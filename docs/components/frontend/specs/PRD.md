---
status: draft
version: "0.3"
date: 2026-10-27
---

# PRD — Insight Portal User Interface

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Shell and Navigation](#51-shell-and-navigation)
  - [5.2 Global View Controls](#52-global-view-controls)
  - [5.3 Home](#53-home)
  - [5.4 Dashboards](#54-dashboards)
  - [5.5 Explore](#55-explore)
  - [5.6 People](#56-people)
  - [5.7 Reports](#57-reports)
  - [5.8 Manage](#58-manage)
  - [5.9 States and Planned Pages](#59-states-and-planned-pages)
  - [5.10 Appearance and Access](#510-appearance-and-access)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 Quality Vector Analysis](#61-quality-vector-analysis)
  - [6.2 NFR Inclusions](#62-nfr-inclusions)
  - [6.3 NFR Exclusions](#63-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Insight portal is the web interface where people read their organisation's engineering, collaboration and AI-usage metrics, build and share dashboards, and administer the installation. This PRD states what the interface must do: navigation and shared controls, each page's job, behaviour when data is missing, refused or still loading, and who may reach what.

It covers the shell and every page reachable from the six rail items: Home, Dashboards, Explore, People, Reports and Manage. Visual tokens, components and layout are in the [technical design](./DESIGN.md).

### 1.2 Background / Problem Statement

The portal grew area by area. Each area chose its own period control, filter placement, placeholder for unbuilt sections, and empty and error states. A reader moving between Home, a direction lens and a custom dashboard meets three period controls with three preset lists. A dashboard opens on a window its owner picked, so it cannot be read over the window the reader just chose elsewhere.

Custom dashboards were an administrator-only workshop: no owner, no sharing, and no way to tell one's own dashboards from anyone else's. The [sharing PRD](../../../domain/insight-v3/specs/sharing/PRD.md) specifies dashboard ownership and view-only sharing; the interface must show them.

A unified visual design now exists for the shell, Home, the dashboard list, an open dashboard, the create-dashboard dialog and the period control. This PRD combines that design with the behaviour of every page the design does not draw, so the interface is specified as one product rather than a set of migrations.

### 1.3 Goals (Business Outcomes)

- Every page that shows period figures uses the same reporting-window control. On a given day, a preset resolves to the same dates on every page.
- Every signed-in person reaches any dashboard shared with them in two clicks from Home. Every dashboard card says whether it is the person's own, shared or a starter.
- No page shows a refused, failed, withheld or not-yet-connected read as an empty result or a zero.
- An installation can hide any navigation item, or mark it planned, by configuration alone, with no rebuild.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Rail | The vertical strip of top-level destinations at the left edge of every screen. |
| Pane | The navigation column beside the rail, listing the destinations inside the selected rail item. |
| Direction | A functional area of work: Development, Collaboration, Knowledge / Wiki, Sales / CRM, Support. |
| Lens | One view of a direction's metrics, such as Flow or Quality. |
| Reporting window | The date range every number on a page is computed over. |
| Preset | A named reporting window, such as Last 30 days or Last quarter. |
| Rolling window | A preset counted back from today, including today. |
| Calendar window | A preset covering the last complete calendar month or quarter. |
| Org scope | The part of the organisation a lead's people views are computed for. |
| Cohort | The group of people each person is compared against. |
| Member | A signed-in person without the administrator role. |
| Evidence | The records and people behind a figure, reachable from the figure. |
| Starter dashboard | A read-only dashboard Insight provides for a connected source. A person can open it or save it as their own. |
| Planned page | A destination shown before it is built, to signal intent, and drawn differently from a working one. |
| Catalogue | The definitions dashboards are built from (metrics, widgets, datasets) and the alerts over them. |
| Assistant | The chat helper that answers data questions and creates or changes definitions. |

## 2. Actors

Reach is whose data each persona may see. [Main Scenarios by Persona §1](../../../product/SCENARIOS.md#1-personas) defines it and the services enforce it. The interface shows that reach and never widens it.

### 2.1 Human Actors

#### Executive

**ID**: `cpt-insightspec-portal-actor-exec`

**Role**: An executive or portfolio leader reading the organisation, a function or a team over time.
**Needs**: Organisation-wide figures with their coverage, comparisons across groups, and evidence behind any figure their grant allows.

#### Team Lead

**ID**: `cpt-insightspec-portal-actor-lead`

**Role**: A functional leader or manager responsible for one or more teams.
**Needs**: Their team at any depth with people by name, what changed over the period, where work stalls, and per-person drill-down within their reach.

#### Individual Contributor

**ID**: `cpt-insightspec-portal-actor-ic`

**Role**: A person in a reporting hierarchy with no reports.
**Needs**: What is visible about themselves and how they compare with their cohort median. Nothing about anyone else.

#### Peer

**ID**: `cpt-insightspec-portal-actor-peer`

**Role**: A member of a flat organisation with no reporting lines.
**Needs**: The organisation's roster and figures, as the flat visibility policy allows.

#### Administrator

**ID**: `cpt-insightspec-portal-actor-admin`

**Role**: A data steward holding the administrator role on top of any persona above.
**Needs**: Connector health, identity resolution, ingestion, portal usage, and the authoring catalogue.

#### Dashboard Owner

**ID**: `cpt-insightspec-portal-actor-owner`

**Role**: An administrator who owns a dashboard or folder, or holds the editor level on one, per [sharing PRD](../../../domain/insight-v3/specs/sharing/PRD.md) revision 0.5.
**Needs**: Create, change, file, tag and share dashboards; see and manage their grants.

#### Dashboard Viewer

**ID**: `cpt-insightspec-portal-actor-viewer`

**Role**: Any signed-in person who sees a dashboard or folder without owning or editing it. Access comes from a grant to them, a grant to everyone, or a folder carrying it.
**Needs**: Find and open what is shared with them; pin and file it; nothing else.

### 2.2 System Actors

#### Insight Services

**ID**: `cpt-insightspec-portal-actor-services`

**Role**: The analytics, identity and definition services behind the portal. They compute every figure and decide every person's reach. They own dashboards, folders, tags, pins and shares, and refuse what a caller may not see or do.

#### Assistant Service

**ID**: `cpt-insightspec-portal-actor-assistant`

**Role**: Answers data questions and creates or replaces metrics, widgets and dashboards on a person's behalf.

#### Installation Configuration

**ID**: `cpt-insightspec-portal-actor-installation`

**Role**: Per-installation settings delivered to the browser. They hide pages or mark them planned, and switch optional features on.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Runs in current evergreen desktop and mobile browsers; nothing is installed on the client.
- Every view is reachable by address. An address copied from one browser opens the same view in another, for a person with the same reach.
- All dates are bucketed in UTC, and the interface says so wherever a reporting window is shown.

## 4. Scope

### 4.1 In Scope

- The shell: rail, pane, top bar, settings, feedback, sign-in and sign-out, and behaviour on phone, tablet and desktop widths.
- Global view controls: reporting window, org scope and cohort.
- Home.
- Dashboards: list, browse filters, folders, tags, pins, open dashboard, create dialog, sharing, starter dashboards, the catalogue (metrics, widgets, datasets, alerts), the definition editor and the Assistant.
- Explore: direction lenses, repository drill-down, organisation-grain lenses, and AI & Cost.
- People: the person page and its sections, the team view, the roster and the work chart.
- Reports: the report builder.
- Manage: every administration destination.
- Loading, empty, error, refusal and planned states across all of the above.
- Light and dark themes, keyboard use and screen-reader use.

### 4.2 Out of Scope

- What a metric means or how it is computed: owned by the metric catalogue.
- Who may see whose data: owned by the services and [Main Scenarios by Persona](../../../product/SCENARIOS.md).
- Dashboard ownership and sharing rules: owned by the [sharing PRD](../../../domain/insight-v3/specs/sharing/PRD.md). This PRD covers only how they are shown.
- Views marked planned in this PRD: Reports snapshots, PDF / HTML export and templates; the planned AI & Cost views; Manage destinations not yet built. Each shows as planned until it has its own requirements.
- A graphical dashboard editor. Dashboards are composed through the definition editor or the Assistant.
- Localisation: per-installation language, date, number, currency and time-zone rules ([S-9](../../../product/SCENARIOS.md#s-9--configuration-and-access)) are deferred. This release is English with UTC days.
- Cohort and attribute slicing for peers in a flat organisation ([S-4](../../../product/SCENARIOS.md#s-4--dashboards-views-and-exploration)) is deferred. The cohort control applies to hierarchical organisations.
- For people without the administrator role: authoring, editing, sharing, the catalogue, the Assistant, alerts and drill-down, per sharing PRD revision 0.5.

## 5. Functional Requirements

### 5.1 Shell and Navigation

#### Six Top-Level Destinations

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-rail`

The system **MUST** show a rail on every screen with Home, Dashboards, Explore, People, Reports and Manage, in that order. It shows only destinations the viewer may reach, marks the current one, and draws a planned destination differently from a working one.

**Rationale**: One fixed set of destinations lets every persona find the same thing in the same place.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Pane per Destination

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-pane`

The system **MUST** show a pane beside the rail listing, in titled groups, the destinations inside the selected rail item. The pane marks the current destination and shows a count beside a destination where the count tells the reader what they will find.

**Rationale**: The pane is where a reader chooses what to look at, so it must say what is there before they open it.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Narrow Screens

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-narrow-screens`

On a phone-width screen the system **MUST**:

- keep every destination reachable through a drawer holding both navigation levels;
- close the drawer when a destination is picked;
- keep the global view controls on one row that scrolls sideways rather than stacking.

**Rationale**: Leads and executives read the portal away from a desk, and a control they cannot reach is a view they cannot use.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Settings and Account

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-settings`

The system **MUST** offer a settings menu from the rail. It shows the signed-in person's name and email, and lets them:

- choose light, dark or system theme;
- show or hide planned pages;
- choose a focus mode;
- turn explanations on or off;
- open What's new;
- sign out.

Each choice persists on that browser.

**Rationale**: A person must see who they are signed in as and be able to end the session from the product itself.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Feedback

- [ ] `p3` - **ID**: `cpt-insightspec-portal-fr-feedback`

The system **MUST** let any signed-in person send free-text feedback from the rail:

- the text is bounded in length, with the remaining length shown;
- the feedback carries the screen it was sent from, but none of that screen's figures or names;
- the system confirms sending or explains the failure.

**Rationale**: Feedback tied to its screen is actionable, but feedback carrying screen data would leak it.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`

#### Session Boundaries

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-session`

The system **MUST**:

- show a person without a session nothing but a sign-in transition;
- tell a person whose sign-in was denied that access was denied, and let them retry where a retry can succeed;
- when a session ends while the portal is open, say so before returning the person to sign-in.

**Rationale**: A silent redirect mid-reading loses the reader's place and looks like a fault.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Unknown Addresses

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-not-found`

When a person opens an address that names no view, or a view that no longer exists, the system **MUST** say the view was not found and offer a way to Home. It **MUST** never silently show a different view.

**Rationale**: A stale shared link that silently lands elsewhere is mistaken for the view it named.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-viewer`

### 5.2 Global View Controls

#### One Reporting Window Control

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-period-control`

The system **MUST** offer the same reporting-window control on every page that shows period figures. The control:

- offers the presets Yesterday, Last 7 days, Last 30 days, Last 90 days, Last month, Last quarter and All time;
- makes the two most used presets reachable directly;
- shows every preset with the dates it resolves to;
- offers a custom range;
- always shows the active dates and their UTC basis.

A preset applies on selection. A custom range applies on confirmation; a range longer than the supported maximum is refused, with the reason.

**Rationale**: One preset list replaces three, and visible dates remove doubt about what a preset means.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`

#### Window Semantics

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-period-semantics`

The system **MUST** resolve presets as below, and to the same dates wherever a preset is shown or computed:

- rolling presets: the stated number of UTC days up to and including today;
- Yesterday: the previous UTC day;
- Last month and Last quarter: the previous complete calendar month and quarter;
- All time: all available history.

**Rationale**: A reader who cannot predict a preset's dates cannot compare it with another report.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Window Scope per Page

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-period-scope`

The system **MUST** keep two kinds of reporting window:

- one portal window, shared by Home, Explore, People and Reports;
- one window per open dashboard, which opens on the dashboard's own default.

Both are kept in the address, and changing one never changes the other. An address without a window opens on the person's last-used portal window, or Last 30 days on a first visit; that window is then written into the address.

**Rationale**: A dashboard is authored for a window and shared by link, while the portal window is the reader's own working context.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-viewer`

#### View Controls Stay Reachable

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-pinned-toolbar`

The system **MUST** keep the reporting-window control visible while the content under it scrolls.

**Rationale**: The window applies to everything on screen, so a reader scrolling a long view must still see and change it.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Org Scope from the Organisation

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-org-scope`

The system **MUST** let a lead or executive narrow people views to a part of the organisation they may see:

- they choose the part from the work chart or from a person's Team action;
- the views it affects show which part is in effect;
- where the chosen part has indirect reports, they choose between direct reports only and the whole subtree.

**Rationale**: Scope is a choice about the organisation, made where the organisation is drawn.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`

#### Cohort

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-cohort`

In a hierarchical organisation, the system **MUST** let a reader choose the cohort every comparison on screen is made against: the whole org scope, or the people who share a person's value of an organisation attribute. The cohort control:

- sits beside the reporting-window control and stays in view while content scrolls;
- explains what the choice controls;
- is shown on every page that draws comparisons;
- offers no choice where only one cohort exists.

**Rationale**: "Versus median" is meaningless unless the reader knows, and can change, whom the median is over.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`

### 5.3 Home

#### Home Composition

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-home`

The system **MUST** show on Home, in this order:

1. starter dashboards for the viewer's connected sources;
2. the three most recently changed dashboards the viewer owns, with a New Dashboard action for people who may author;
3. per-person headline figures for the period, with their change against the previous window;
4. what needs attention, grouped by metric, with the number of people affected;
5. AI adoption;
6. a card per direction, leading to it.

**Rationale**: Home answers "what should I look at" before "what are the numbers".

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-peer`

#### Home Pane

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-home-pane`

The system **MUST** list in Home's pane:

- the overview themes At a glance, By direction, Trend, Attention needed, Data coverage and Contribution breakdown;
- a Dashboards group linking to My dashboards and Starter dashboards, with their counts.

**Rationale**: The shortest path from landing to a saved dashboard starts on Home.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-peer`

### 5.4 Dashboards

#### Dashboards for Everyone

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-dashboards-access`

The system **MUST** open Dashboards to every signed-in person, showing each person only the dashboards, folders and pins the services return for them. It **MUST** offer creating, editing, sharing, the catalogue, alerts and the Assistant only to administrators, as sharing PRD revision 0.5 allows.

**Rationale**: Sharing is pointless if the people it is shared with cannot reach it.

**Actors**: `cpt-insightspec-portal-actor-viewer`, `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-admin`

#### Browse Filters

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-browse`

The system **MUST** let a person narrow the dashboard list to All, My dashboards, Shared with me, Starred or Starter, each with its count. The filter is offered both in the pane and as a row of chips above the list, and the two **MUST** always show the same selection.

**Rationale**: "Mine", "shared with me" and "starter" are the three questions a person brings to a list of dashboards.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Dashboard List

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-dashboard-list`

The system **MUST** list dashboards as cards. Each card shows:

- the name;
- the owner, where identity lets the person read the owner, and otherwise no owner;
- how long ago it changed;
- whether it is private, shared with people or shared with everyone, naming the grants only to people who manage them;
- its tags;
- an open action and a menu of what the person may do with it.

The list **MUST** offer search by name and a tag filter, show the matching count, and load further pages as the person scrolls.

**Rationale**: Owner, age and visibility tell a person whether a dashboard is theirs, current and safe to share onward.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Starter Dashboards

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-starter`

The system **MUST** offer read-only starter dashboards for the viewer's connected sources. Each starter:

- states the question it answers and the sources it reads, with their connection state;
- can only be opened from the list;
- can be saved as their own dashboard by people who may author.

**Rationale**: A new person needs a working dashboard before they know what to build.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Starred

- [ ] `p3` - **ID**: `cpt-insightspec-portal-fr-starred`

The system **MUST** let a person star and unstar any dashboard they can see from its menu, and list their starred dashboards under Starred.

**Rationale**: A person reading many shared dashboards needs a short list of the ones they return to.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Folders, Tags and Pins

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-organise`

As the services permit, the system **MUST** let a person:

- create, rename and delete folders;
- place a dashboard in folders;
- tag dashboards they may tag;
- pin dashboards to the pane.

It **MUST** show folders and pinned dashboards in the pane, and say what happens to a folder's dashboards before deleting the folder.

**Rationale**: Organising is what turns a list into a workspace.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Open Dashboard

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-open-dashboard`

The system **MUST** show for an open dashboard:

- its name;
- its owner and visibility, as in `cpt-insightspec-portal-fr-dashboard-list`;
- the sources it reads, with their connection state;
- when it last changed;
- its widgets, in the stored order;
- its reporting window, per `cpt-insightspec-portal-fr-period-scope`;
- an Edit action, for people who may edit it;
- a menu offering, as permitted, Duplicate, Share, Rename, Pin, Star, Move to folder, Tags and Delete, with Export snapshot and Schedule report shown as planned.

**Rationale**: A reader must know whose view this is, what it reads and how fresh it is before trusting it.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Widget Reading

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-widgets`

The system **MUST** draw each widget with:

- its title;
- a subtitle naming its aggregation, grain and unit, where it has them;
- the figure or chart;
- for people the services allow to drill down, a way to see the rows behind it.

A headline figure **MUST** show its change against the previous window of the same length. A widget that fails **MUST** show a member a generic failure without its cause.

**Rationale**: A chart without its unit and grain, or a number without its comparison, cannot be read correctly.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Create Dashboard

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-create-dashboard`

The system **MUST** let a person who may author create a dashboard from a dialog reachable from Home and Dashboards. The dialog asks for:

- a name;
- who can see it: Private (the default), named people, or everyone;
- a starting point: empty, a copy of a starter dashboard, or a description handed to the Assistant with the name and visibility already set.

On creation, the system **MUST** open the new dashboard.

**Rationale**: One small step replaces a blank technical form, and describing a dashboard to the Assistant stays a first-class way to make one.

**Actors**: `cpt-insightspec-portal-actor-owner`

#### Share

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-share`

For a person who may share a dashboard or folder, the system **MUST**:

- let them grant it to named people or to everyone, at the viewer or editor level;
- let them change a grant's level, and list and withdraw grants;
- never preselect everyone;
- warn before any action that widens who sees a dashboard: a grant, or placing the dashboard in a shared folder.

The system **MUST** offer Share only where the person may share.

**Rationale**: Sharing must be deliberate, visible and reversible.

**Actors**: `cpt-insightspec-portal-actor-owner`

#### Orphaned Dashboards and Folders

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-orphaned`

The system **MUST** list, for administrators, the dashboards and folders whose owner no longer holds the administrator role, with name, owner and last change. It **MUST** let an administrator make a current administrator their owner.

**Rationale**: Work must stay in use when its owner changes role.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Catalogue

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-catalogue`

The system **MUST** list metrics, widgets, datasets and alerts in a Catalogue group under Dashboards:

- each list is searchable and paged;
- each item opens a page showing its definition, a run of it over a chosen window (where it is dated), and what uses it.

**Rationale**: A dashboard is only as trustworthy as the definitions under it, and a person must see what a definition feeds before changing it.

**Actors**: `cpt-insightspec-portal-actor-admin`, `cpt-insightspec-portal-actor-owner`

#### Definition Editor

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-editor`

The system **MUST** let a person who may edit a definition change it through a form or as text, and:

- refuse invalid input field by field, with the reason;
- never lose unsaved input on a refusal;
- refuse to remove a definition while something still uses it, naming what does.

**Rationale**: Authoring is error-prone, and a refusal that discards work or does not say why stops people authoring.

**Actors**: `cpt-insightspec-portal-actor-admin`, `cpt-insightspec-portal-actor-owner`

#### Alerts

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-alerts`

The system **MUST** let a person who may author:

- define an alert over a metric: its condition, window, check interval, columns to include and destination;
- preview the metric's current value while defining it;
- turn the alert on and off;
- see its latest check and notification history.

It **MUST** say when no destination is configured.

**Rationale**: An alert whose last check a person cannot see is not trusted.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Assistant

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-assistant`

The system **MUST** offer the Assistant beside every Dashboards page to people who may author. The Assistant:

- does not take the reader's screen until asked for;
- keeps the conversation while the person moves between Dashboards pages;
- says in each reply whether it answered or built something, shows any result rows, and links what it built;
- on a refused request, shows the reason and returns the question to the input.

**Rationale**: Describing what one wants is how most dashboards are made, so the Assistant must be at hand without taking the screen.

**Actors**: `cpt-insightspec-portal-actor-admin`, `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-assistant`

### 5.5 Explore

#### Directions and Lenses

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-lenses`

The system **MUST** list the directions with their lens counts, and open each lens as a page of sections over the org scope and reporting window. A lens page **MUST** show the number of people its figures cover. Its sections are headline figures with change, trends, distributions, concentration, composition and any other kinds the lens defines.

**Rationale**: A direction is read lens by lens, and each lens must say how many people it is about.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-peer`

#### Evidence Behind a Figure

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-evidence`

From any figure, band, trend point or cell, the system **MUST** let a reader:

- open the people behind it, where they hold the person-level grant;
- open the records behind it, where they hold the underlying-records grant ([rule 5](../../../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario));
- drill from a person to their records in place;
- export what is shown as CSV or Excel.

Without the grant, the figure **MUST** say its evidence is not available to them.

**Rationale**: A figure that cannot be followed back is not acted on.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Repository Drill-Down

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-repository`

From a repository row, the system **MUST** open that repository's figures, filtered to it, with a breadcrumb back to the lens. The figures are headline, contributors, activity by hour, composition by branch, ownership and event histogram.

**Rationale**: Delivery problems are often specific to one repository.

**Actors**: `cpt-insightspec-portal-actor-lead`

#### Organisation-Grain Lenses

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-org-grain`

The system **MUST** show lenses whose metrics belong to the organisation rather than to people, such as CI pipelines, without a roster. It **MUST** say when the installation has not enabled those metrics.

**Rationale**: Some work has no person attached, and forcing it into a per-person shape misleads.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### AI and Cost

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-ai-cost`

To readers holding the cost grant, the system **MUST** show AI adoption and spend for the org scope, keeping seat-based and usage-based cost apart:

- potential cost, active users, accepted lines and average cost per active user;
- cost by tool;
- cost and usage per person;
- an adoption funnel;
- a breakdown by the chosen cohort attribute.

A reading the source does not provide **MUST** show as not tracked, never as zero.

**Rationale**: Finance and leads decide on AI spend from these figures, and a missing reading shown as zero understates cost.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

### 5.6 People

#### Person Page

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-person`

For a person the viewer may see, the system **MUST** show:

- their name, title and department;
- their manager, where the manager is within the viewer's reach;
- which sections have data, which have none and which have no connected source;
- headline figures with trends against their cohort;
- what needs attention;
- per section, the figures and the work behind them.

**Rationale**: "What is visible about me" is the individual contributor's first question and a lead's drill-down target.

**Actors**: `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`

#### Team View

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-team`

For the org scope, the system **MUST** show its size, team-level summary figures, what needs attention, and every member in a grid compared within the cohort. The grid **MUST** be ordered by name until the reader chooses another order, and let the reader show a subset of members and open any member's person page.

**Rationale**: A lead's period review starts with the team and ends with a person.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-peer`

#### Roster

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-roster`

The system **MUST** list every person the viewer may see, with their title, organisational placement and status. The list:

- is searchable;
- is grouped by role where its name says so;
- opens the person page from each entry;
- says when nobody matches.

**Rationale**: Finding a person is the most common way into their data.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-exec`

#### Work Chart

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-work-chart`

The system **MUST** draw, in the People pane, the reporting tree rooted at the viewer, or an alphabetical list in a flat organisation. The chart **MUST** be searchable, and selecting an entry opens a lead's team or a person's page.

**Rationale**: The organisation's shape is how leads navigate their people.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-peer`

### 5.7 Reports

#### Report Builder

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-report-builder`

The system **MUST** let a reader:

- pick metrics by family;
- choose people or organisation grain, and a time granularity the reporting window supports;
- preview the resulting rows;
- download them as CSV or Excel;
- cancel a running preparation.

When the preview cannot run, the system **MUST** say why.

**Rationale**: Figures leave the product in spreadsheets, so the export must be the same data the portal shows.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

### 5.8 Manage

#### Administration Destinations

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-manage`

The system **MUST** group administration as follows:

- **Data**: Sources & connectors, Data exclusions, Org snapshots;
- **People & access**: Identities · roles & taxonomy, Access, Group management;
- **Platform**: Ingestion, Previews, AI assistant config, What's new, Platform usage, MCP servers, Config & setup, Scorecard management.

It **MUST** show each destination only to people who may use it, and open Manage on a working destination.

**Rationale**: Administrators need one place for the installation's health and configuration, and landing on a placeholder wastes their first click.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Connector and Ingestion Health

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-ingestion-health`

For each connector, the system **MUST** show its state, last sync, records reported and recent syncs, and the rate rows arrive over recent and live windows. It **MUST** distinguish a connector never read from one read with nothing recorded.

**Rationale**: Every metric depends on ingestion, so an administrator must see a stalled source before readers see wrong numbers.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Identity Resolution

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-identities`

The system **MUST** let an administrator work a review queue of unresolved accounts (merge, absorb, confirm) and look up a person's accounts or an account's owner.

**Rationale**: A person's figures are only as good as the accounts attributed to them.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Portal Usage

- [ ] `p3` - **ID**: `cpt-insightspec-portal-fr-usage`

The system **MUST** show administrators visits, people and pages opened over the reporting window; who opened what; which drill-downs and exports were used; and the feedback people sent.

**Rationale**: Adoption decides which pages to invest in.

**Actors**: `cpt-insightspec-portal-actor-admin`

### 5.9 States and Planned Pages

#### Distinct States

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-states`

On every page and every independently loaded section, the system **MUST** distinguish these states and show each differently:

- loading;
- no data for this reader;
- no connected source;
- withheld because the group is below the size a figure or comparison needs ([rule 10](../../../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario)), with the reason;
- a refusal, with its reason;
- a failure, with a retry.

**Rationale**: "No data" and "could not load" lead to opposite conclusions.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-admin`

#### Viewing as Another Person

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-view-as`

While an administrator views the portal as another person, the system **MUST** show on every screen whose view it is and a way back to their own. It **MUST** mark every screen drawn from demonstration data as such.

**Rationale**: An administrator acting under another person's reach, or a reader looking at demonstration figures, must never mistake them for their own.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Planned Pages

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-planned`

The system **MUST**:

- hide planned pages unless the reader turns them on;
- mark them as planned where shown;
- never make a planned page the default destination;
- let an installation hide any page or mark it planned.

**Rationale**: Signalling intent must not cost a reader a dead end, and each installation offers a different set of sources.

**Actors**: `cpt-insightspec-portal-actor-installation`, `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Destructive Actions

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-destructive`

Before every deletion and every action that widens who sees something, the system **MUST** ask for confirmation, stating the consequence. It **MUST** show deletion as destructive.

**Rationale**: Deletions and widened sharing cannot be quietly undone.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-admin`

### 5.10 Appearance and Access

#### Themes

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-themes`

The system **MUST** render every page in a light and a dark theme, with colour conveying the same meaning in both, and follow the operating system's choice when the person picks system.

**Rationale**: Readers work in both themes, and a status that disappears in one is a status lost.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Keyboard and Screen Reader

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-a11y`

The system **MUST**:

- let every action be performed from the keyboard, with visible focus;
- offer a skip to the main content;
- announce the current destination in both navigation levels;
- give every control an accessible name;
- announce loading, refusals and errors.

**Rationale**: The portal is a workplace tool, and an inaccessible one excludes colleagues.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

## 6. Non-Functional Requirements

### 6.1 Quality Vector Analysis

| Quality Vector | Show-Stopper Requirement | Rationale |
|-----------------|---------------------------|-----------|
| Efficiency | None; covered by `cpt-insightspec-portal-nfr-bounded-requests` | Requests that grow with a list slow readers and load the services, but do not make the portal unusable. |
| Reliability | The system **MUST** never show a failed, refused or unconnected read as empty or zero (`cpt-insightspec-portal-nfr-honest-states`). | A lead who reads a failed load as "no activity" misjudges named people and stops trusting every other figure. |
| Performance | None; covered by `cpt-insightspec-portal-nfr-feedback-latency` | The services compute figures; the interface must acknowledge every action promptly so a wait is never mistaken for a fault. |
| Security | The system **MUST** never draw or export data, names or destinations outside the viewer's reach and grants (`cpt-insightspec-portal-nfr-reach`, `cpt-insightspec-portal-nfr-exports`). | Showing a person's figures to someone outside their reach is a privacy breach that ends the product's use in an organisation. |
| Versatility | None; covered by `cpt-insightspec-portal-nfr-accessibility`, `cpt-insightspec-portal-nfr-screen-widths`, `cpt-insightspec-portal-nfr-installation-policy` | Installations differ in sources, organisation shape, devices and needs; each is covered, and none alone makes the portal unviable. |

### 6.2 NFR Inclusions

#### Honest States

- [ ] `p1` - **ID**: `cpt-insightspec-portal-nfr-honest-states`

**Vector**: Reliability

The system **MUST** show a failed, refused or not-connected read differently from an empty one, and show a reading a source does not provide as not tracked rather than zero.

**Threshold**: Zero pages where a failed or refused read renders as an empty list, an empty chart or a zero.

**Rationale**: Conclusions about people are drawn from what the screen shows.

#### Reach Never Widened

- [ ] `p1` - **ID**: `cpt-insightspec-portal-nfr-reach`

**Vector**: Security

The system **MUST** show only what the services returned for the viewer, and show a refusal rather than an empty view when the services refuse. It **MUST** treat hiding a destination as a convenience, never as the access control.

**Threshold**: No figure, name or destination is drawn that the services did not return for the signed-in person.

**Rationale**: The services enforce reach, and the interface must not leak around them or suggest data exists where it is refused.

#### Bounded Requests per List

- [ ] `p2` - **ID**: `cpt-insightspec-portal-nfr-bounded-requests`

**Vector**: Efficiency

The system **MUST** show a page of any list (dashboards, catalogue items, people) with work that does not grow with the number of items on the page.

**Threshold**: The time and service work to show a list page do not depend on how many items it holds, up to the page size.

**Rationale**: Per-item requests make the busiest lists the slowest and multiply load on the services.

#### Prompt Feedback

- [ ] `p2` - **ID**: `cpt-insightspec-portal-nfr-feedback-latency`

**Vector**: Performance

The system **MUST** acknowledge every action visibly before any wait, and show a loading state for any wait longer than a moment.

**Threshold**: A loading state appears within 200 ms of any action whose result is not yet available. A page-load budget is an open decision owned by the product owner.

**Rationale**: An unacknowledged click is repeated or abandoned.

#### Accessibility

- [ ] `p1` - **ID**: `cpt-insightspec-portal-nfr-accessibility`

**Vector**: Versatility

The system **MUST** meet WCAG 2.2 level AA on every page in both themes.

**Threshold**: WCAG 2.2 AA conformance, including text contrast of at least 4.5:1 and non-text contrast of at least 3:1.

**Rationale**: Colleagues with disabilities must be able to read and act on the same figures.

#### Screen Widths

- [ ] `p2` - **ID**: `cpt-insightspec-portal-nfr-screen-widths`

**Vector**: Versatility

The system **MUST** keep every page usable from a 360 px-wide phone screen to a desktop screen without horizontal page scrolling.

**Threshold**: No horizontal page scroll at widths of 360 px and above; wide tables and charts scroll within their own frame.

**Rationale**: Leads read on phones between meetings.

#### Installation Policy

- [ ] `p2` - **ID**: `cpt-insightspec-portal-nfr-installation-policy`

**Vector**: Versatility

The system **MUST** let an installation hide, or mark as planned, any rail destination, pane destination, direction, lens, person-page section or metric through configuration, without rebuilding the portal.

**Threshold**: Every navigation item and every metric can be hidden or marked planned by configuration alone.

**Rationale**: Installations connect different sources and roll pages out at different times.

#### Exports Carry Only What Is Shown

- [ ] `p1` - **ID**: `cpt-insightspec-portal-nfr-exports`

**Vector**: Security

The system **MUST** put into a CSV or Excel export only the rows the reader sees on screen under their grants. It **MUST** record every export as a usage event naming the page, never the export's contents.

**Threshold**: An export never contains a person, record or figure the reader could not see on screen, and every export appears in portal usage.

**Rationale**: An exported file leaves the product's access rules, so it must not carry more than the reader already had.

### 6.3 NFR Exclusions

- Offline use: the portal shows live figures from the services, and a cached offline view would show stale figures as current.
- Localisation: English is the only interface language in scope (§4.2).
- Availability and recovery: the portal is a static bundle with no state of its own, so it inherits availability from the gateway and the services.
- Authentication and session lifetime: owned by the [authenticator PRD](../../backend/authenticator/PRD.md). The portal only shows session boundaries (`cpt-insightspec-portal-fr-session`).
- Personal-data retention, erasure and processing records: owned by the services that store the data. The portal stores no personal data beyond per-browser preferences.
- Scale targets (people per installation, dashboards per person): no portal-specific target. Lists are paged and bounded by `cpt-insightspec-portal-nfr-bounded-requests`; an installation-size target is an open decision owned by the product owner.
- Operator documentation: the portal's only operator-facing configuration is the installation policy, documented with the deployment chart.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Portal Addresses

- [ ] `p1` - **ID**: `cpt-insightspec-portal-interface-addresses`

**Type**: Data format: the portal's web addresses.

**Stability**: stable

**Description**: Every view is named by its address, together with its reporting window, org scope, cohort, folder and tag filters. An address can therefore be bookmarked, shared and reopened.

**Breaking Change Policy**: A shared address keeps opening the view it named, or a not-found view per `cpt-insightspec-portal-fr-not-found`. It never silently opens a different view.

### 7.2 External Integration Contracts

#### Insight Service Contracts

- [ ] `p1` - **ID**: `cpt-insightspec-portal-contract-services`

**Direction**: required from client

**Protocol/Format**: The analytics, identity, definition, sharing and assistant service interfaces, as specified by their own designs.

**Compatibility**: The portal is released with the services it calls. A service's refusal is shown with its reason, never reinterpreted.

#### Installation Configuration Contract

- [ ] `p2` - **ID**: `cpt-insightspec-portal-contract-installation`

**Direction**: required from client

**Protocol/Format**: Per-installation configuration delivered to the browser at load. It names the pages to hide, the pages to mark planned, and the optional features enabled.

**Compatibility**: An entry the portal does not recognise is ignored, never fatal.

## 8. Use Cases

#### Review a Team over a Period

- [ ] `p1` - **ID**: `cpt-insightspec-portal-usecase-team-review`

**Actor**: `cpt-insightspec-portal-actor-lead`

**Preconditions**:
- The lead is signed in and manages at least one team.

**Main Flow**:
1. The lead opens People and picks their team in the work chart.
2. The lead picks Last quarter.
3. The team view shows summary figures, what needs attention and every member.
4. The lead opens an attention item and follows a member's figure to its evidence.

**Postconditions**:
- The lead has seen the team over the last complete quarter, with the people behind each flag.

**Alternative Flows**:
- **A section's source is not connected**: the section says so instead of showing an empty chart.

#### Build a Dashboard by Describing It

- [ ] `p1` - **ID**: `cpt-insightspec-portal-usecase-describe-dashboard`

**Actor**: `cpt-insightspec-portal-actor-owner`

**Preconditions**:
- The person may author dashboards.

**Main Flow**:
1. The person chooses New Dashboard, names it, keeps it Private and chooses to describe it to the Assistant.
2. The Assistant opens with the name and visibility set, and the person describes what they want.
3. The Assistant builds the dashboard and links it.
4. The person opens the dashboard.

**Postconditions**:
- A private dashboard owned by the person exists and is open.

**Alternative Flows**:
- **The Assistant refuses**: the reason is shown and the description returns to the input.

#### Share a Dashboard

- [ ] `p1` - **ID**: `cpt-insightspec-portal-usecase-share`

**Actor**: `cpt-insightspec-portal-actor-owner`

**Preconditions**:
- The person may share the dashboard.

**Main Flow**:
1. The person opens the dashboard's menu and chooses Share.
2. The person grants view access to two named colleagues.
3. The dashboard's visibility line names the grant.

**Postconditions**:
- The two colleagues see the dashboard under Shared with me.

**Alternative Flows**:
- **Granting to everyone**: the portal warns that everyone will see it and asks to confirm.

#### Open a Dashboard Shared with Me

- [ ] `p1` - **ID**: `cpt-insightspec-portal-usecase-open-shared`

**Actor**: `cpt-insightspec-portal-actor-viewer`

**Preconditions**:
- A dashboard is shared with the person, who does not hold the administrator role.

**Main Flow**:
1. The person opens Dashboards and chooses Shared with me.
2. The person opens the dashboard.
3. The dashboard shows its owner where identity allows, its widgets over its own window, and a menu offering only pin and file.

**Postconditions**:
- The person has read the dashboard, with no editing, sharing or drill-down action offered.

**Alternative Flows**:
- **The share is withdrawn while open**: the next read shows that the dashboard is no longer available to them.

#### Check Source Health

- [ ] `p2` - **ID**: `cpt-insightspec-portal-usecase-source-health`

**Actor**: `cpt-insightspec-portal-actor-admin`

**Preconditions**:
- The person holds the administrator role.

**Main Flow**:
1. The administrator opens Manage, which opens on a working destination.
2. The administrator opens Sources & connectors and expands a connector's recent syncs.
3. The administrator opens Ingestion and drills into the same connector's arrival rate.

**Postconditions**:
- The administrator knows whether the connector is syncing and how fast rows arrive.

**Alternative Flows**:
- **The connector was never read**: the page says so, distinct from a connector read with nothing recorded.

#### See What Is Visible About Me

- [ ] `p1` - **ID**: `cpt-insightspec-portal-usecase-self-view`

**Actor**: `cpt-insightspec-portal-actor-ic`

**Preconditions**:
- The individual contributor is signed in.

**Main Flow**:
1. The person opens People and chooses Me.
2. The person page shows their figures against their cohort median, and which sections have no source.

**Postconditions**:
- The person has seen their own figures and nobody else's.

**Alternative Flows**:
- **Fewer people in the cohort than the comparison needs**: the comparison is withheld and the page says why.

## 9. Acceptance Criteria

- [ ] Home, Explore, People and Reports show the same reporting-window control with the same presets, and the same dates for the same preset on the same day.
- [ ] An open dashboard keeps its own reporting window in its address, and changing it does not change the window on Home.
- [ ] A person without the administrator role who has a dashboard shared with them finds it under Shared with me and opens it.
- [ ] Forcing any section's read to fail shows a retryable error, never an empty state.
- [ ] Every page passes an automated WCAG 2.2 AA check in both themes, and a keyboard-only walkthrough reaches every action.
- [ ] Manage opens on a working destination for an administrator.
- [ ] An address naming a removed dashboard shows a not-found view.
- [ ] Each rolling preset covers its stated number of days up to and including today, and Last month on any day covers exactly the previous calendar month.
- [ ] A person outside the administrator role sees no create, edit, share, catalogue, Assistant or drill-down action anywhere in Dashboards.
- [ ] A dashboard card shown to a viewer never names the dashboard's other viewers.
- [ ] A team or comparison below the group-size threshold shows a withheld state with its reason, never a figure.
- [ ] The team grid lists members by name until the reader sorts it.
- [ ] Evidence and exports never include a person or record the reader lacks the grant for.
- [ ] Ending a session while the portal is open shows a session-ended notice before sign-in.
- [ ] The settings menu signs the person out.
- [ ] Viewing as another person shows whose view it is on every screen, with a way back.
- [ ] Every page at 360 px width shows no horizontal page scroll.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Analytics service | Computes every figure, trend, distribution and evidence list | p1 |
| Identity service | Decides each person's reach, roles and organisation placement | p1 |
| Definition service | Stores metrics, widgets, dashboards, datasets, alerts, folders, tags, pins | p1 |
| Sharing | Ownership; viewer and editor grants to people or everyone; shared folders; orphaned transfer ([sharing PRD](../../../domain/insight-v3/specs/sharing/PRD.md) revision 0.5) | p1 |
| Window presets on the services | Resolve every preset in `cpt-insightspec-portal-fr-period-control`, including Last 90 days, per `cpt-insightspec-portal-fr-period-semantics` | p1 |
| Assistant service | Answers questions and builds definitions | p2 |
| Installation configuration | Hidden and planned pages, optional features | p2 |

## 11. Assumptions

- The services return each person's reach already applied; the portal never filters a broader result down.
- The services provide starred and starter dashboards; the interface derives neither.
- The services decide which cohort attributes exist, from the organisation's data.
- The product owner accepts a WCAG 2.2 AA target; a different target would be recorded as a revision of `cpt-insightspec-portal-nfr-accessibility`.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Starred and starter dashboards have no service support yet | Browse filters and Home sections have nothing to show | Show them as planned per `cpt-insightspec-portal-fr-planned` until the services provide them |
| Removing the top-bar scope control hides scope from leads who used it | Leads lose track of which part of the organisation a view covers | Every scoped view names its scope (`cpt-insightspec-portal-fr-org-scope`); the work chart sets it |
| Existing dashboards carry a window restriction the single control ignores | A dashboard opens on a window its author did not intend | The dashboard's own default window opens first (`cpt-insightspec-portal-fr-period-scope`) |
| A metric refuses windows longer than its maximum | A widget read over a long preset shows a refusal | The widget shows the refusal with its reason (`cpt-insightspec-portal-fr-states`) |
| The sharing PRD revision this PRD relies on is not merged yet | Requirements on grants, editors and members describe behaviour not yet available | Ship the dashboard requirements with the sharing release; until then show those actions as planned |
