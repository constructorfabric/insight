---
status: draft
version: "0.1"
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
  - [5.9 States and Planned Surfaces](#59-states-and-planned-surfaces)
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

The Insight portal is the web interface through which people read their organisation's engineering, collaboration and AI-usage metrics, build and share dashboards over that data, and administer the installation. This PRD states what the whole interface must do: its navigation, the controls shared across screens, each screen's job, how it behaves when data is missing, refused or still loading, and who may reach what.

It covers every surface reachable from the six-item rail — Home, Dashboards, Explore, People, Reports and Manage — and the shell around them. Visual tokens, components and layout are specified in the [technical design](./DESIGN.md).

### 1.2 Background / Problem Statement

The portal grew area by area. Each area chose its own period control, its own placement of global filters, its own way to show a section that is not built yet, and its own empty and error states. A reader moving between Home, a direction lens and a custom dashboard meets three different period controls with three different preset lists, and a dashboard that opens on a window its owner picked cannot be read over the window the reader just chose elsewhere.

Custom dashboards were an administrator-only workshop with no owner, no sharing and no way to tell one's own dashboards from anyone else's. Dashboard ownership and view-only sharing are specified in the [sharing PRD](../../../domain/insight-v3/specs/sharing/PRD.md); the interface has to present them.

A unified visual design now exists for the shell, Home, the dashboard list, an open dashboard, the create-dashboard dialog and the period control. This PRD combines that design with the behaviour of every surface the design does not draw, so the interface is specified as one product rather than a set of migrations.

### 1.3 Goals (Business Outcomes)

- Every surface that shows period figures uses the same reporting-window control, and the same preset resolves to the same dates on every one of them on the same day.
- Every signed-in person reaches every dashboard shared with them in two clicks from Home, and every dashboard card states whether it is the person's own, shared or a starter.
- No surface presents a refused, failed, withheld or not-yet-connected read as an empty result or a zero.
- An installation hides or marks planned any navigable surface by configuration alone, with no rebuild.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Rail | The narrow vertical strip of top-level destinations at the left edge of every screen. |
| Pane | The navigation column beside the rail listing the destinations inside the selected rail item. |
| Direction | A functional area of work — Development, Collaboration, Knowledge / Wiki, Sales / CRM, Support. |
| Lens | One view of a direction's metrics, such as Flow or Quality. |
| Reporting window | The date range every number on a surface is computed over. |
| Preset | A named reporting window, such as Last 30 days or Last quarter. |
| Rolling window | A preset counted back from today, including today. |
| Calendar window | A preset covering the last complete calendar month or quarter. |
| Org scope | The part of the organisation a lead's people views are computed for. |
| Cohort | The group of people each person is compared against. |
| Evidence | The records and people behind a figure, reachable from the figure. |
| Starter dashboard | A read-only dashboard Insight provides for a connected source, which a person can open or save as their own. |
| Planned surface | A destination shown to signal intent before it is built, distinguishable from a working one. |
| Catalogue | The definitions dashboards are built from — metrics, widgets, datasets — and alerts over them. |
| Assistant | The conversational helper that answers data questions and creates or changes definitions. |

## 2. Actors

Reach — whose data each persona may see — is defined in [Main Scenarios by Persona §1](../../../product/SCENARIOS.md#1-personas) and enforced by the services. The interface presents that reach; it never widens it.

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
**Needs**: What is visible about themselves and how they compare with their cohort median — and nothing about anyone else.

#### Peer

**ID**: `cpt-insightspec-portal-actor-peer`

**Role**: A member of a flat organisation with no reporting lines.
**Needs**: The organisation's roster and figures as the flat visibility policy allows.

#### Administrator

**ID**: `cpt-insightspec-portal-actor-admin`

**Role**: A data steward holding the administrator role on top of any of the personas above.
**Needs**: Connector health, identity resolution, ingestion, usage of the portal, and the authoring catalogue.

#### Dashboard Owner

**ID**: `cpt-insightspec-portal-actor-owner`

**Role**: An administrator who owns a dashboard or folder, or holds the editor level on one, as defined by the [sharing PRD](../../../domain/insight-v3/specs/sharing/PRD.md) revision 0.5.
**Needs**: Create, change, file, tag and share dashboards; see and manage their grants.

#### Dashboard Viewer

**ID**: `cpt-insightspec-portal-actor-viewer`

**Role**: Any signed-in person who sees a dashboard or folder without owning or editing it — through a grant to them, a grant to everyone, or a folder carrying it.
**Needs**: Find and open what is shared with them; pin and file it; nothing else.

### 2.2 System Actors

#### Insight Services

**ID**: `cpt-insightspec-portal-actor-services`

**Role**: The analytics, identity and definition services behind the portal. They compute every figure, decide every person's reach, own dashboards, folders, tags, pins and shares, and refuse what a caller may not see or do.

#### Assistant Service

**ID**: `cpt-insightspec-portal-actor-assistant`

**Role**: Answers data questions and creates or replaces metrics, widgets and dashboards on a person's behalf.

#### Installation Configuration

**ID**: `cpt-insightspec-portal-actor-installation`

**Role**: Per-installation settings delivered to the browser that hide surfaces or mark them as planned, and switch optional features on.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Runs in current evergreen desktop and mobile browsers; no installation on the client.
- Every view is reachable by address: an address copied from one browser opens the same view in another for a person with the same reach.
- All dates are bucketed in UTC, and the interface says so wherever a reporting window is shown.

## 4. Scope

### 4.1 In Scope

- The shell: rail, pane, top bar, settings, feedback, sign-in and sign-out, and behaviour on phone, tablet and desktop widths.
- Global view controls: the reporting window, org scope and cohort.
- Home.
- Dashboards: list, browse filters, folders, tags, pins, open dashboard, create dialog, sharing, starter dashboards, the catalogue (metrics, widgets, datasets, alerts), the definition editor and the Assistant.
- Explore: direction lenses, repository drill-down, organisation-grain lenses, and AI & Cost.
- People: the person page and its sections, the team view, the roster and the work chart.
- Reports: the report builder.
- Manage: every administration destination.
- Loading, empty, error, refusal and planned states across all of the above.
- Light and dark themes, keyboard use and screen-reader use.

### 4.2 Out of Scope

- What any metric means or how it is computed — owned by the metric catalogue.
- Who may see whose data — owned by the services and [Main Scenarios by Persona](../../../product/SCENARIOS.md).
- Dashboard ownership and sharing rules — owned by the [sharing PRD](../../../domain/insight-v3/specs/sharing/PRD.md); this PRD covers only how they are presented.
- The views marked planned in this PRD: Reports snapshots, PDF / HTML export and templates; the planned AI & Cost views; Manage destinations not yet built. Each is shown as planned until its own requirements exist.
- A graphical dashboard editor; dashboards are composed through the definition editor or the Assistant.
- Localisation: per-installation language, date, number, currency and time-zone rules ([S-9](../../../product/SCENARIOS.md#s-9--configuration-and-access)) are deferred; this release is English with UTC days.
- Cohort and attribute slicing for peers in a flat organisation ([S-4](../../../product/SCENARIOS.md#s-4--dashboards-views-and-exploration)) is deferred; the cohort control applies to hierarchical organisations.
- Authoring, editing, sharing, the catalogue, the Assistant, alerts and drill-down for people without the administrator role, per the sharing PRD revision 0.5.

## 5. Functional Requirements

### 5.1 Shell and Navigation

#### Six Top-Level Destinations

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-rail`

The system **MUST** offer, on every screen, a rail with Home, Dashboards, Explore, People, Reports and Manage in that order, showing only the destinations the viewer may reach, marking the current one, and drawing a planned destination distinguishably from a working one.

**Rationale**: One fixed set of destinations lets every persona find the same thing in the same place.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Pane per Destination

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-pane`

The system **MUST** show, beside the rail, a pane listing the destinations inside the selected rail item in titled groups, marking the current one, and showing a count beside a destination where the count tells the reader what they will find.

**Rationale**: The second level of navigation is where a reader chooses what to look at; it must say what is there before they open it.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Narrow Screens

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-narrow-screens`

The system **MUST** keep every destination reachable on a phone-width screen through a drawer holding both levels of navigation, close the drawer when a destination is picked, and keep the global view controls on one row that scrolls sideways rather than stacking.

**Rationale**: Leads and executives read the portal away from a desk; a control they cannot reach is a view they cannot use.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Settings and Account

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-settings`

The system **MUST** offer, from the rail, a settings menu showing the signed-in person's name and email, and letting them choose light, dark or system theme, turn planned surfaces on or off, choose a focus mode and turn explanations on or off, open What's new, and sign out. Each choice persists on that browser.

**Rationale**: A person must be able to see who they are signed in as and end the session from the product itself.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Feedback

- [ ] `p3` - **ID**: `cpt-insightspec-portal-fr-feedback`

The system **MUST** let any signed-in person send free-text feedback from the rail, bounded in length with the remaining length shown, attaching the screen it was sent from without any figures or names on that screen, and confirm or explain failure.

**Rationale**: Feedback tied to the screen it concerns is actionable; feedback carrying screen data would leak it.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`

#### Session Boundaries

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-session`

The system **MUST** show nothing but a sign-in transition to a person without a session, tell a person whose sign-in was denied that access was denied and let them retry where a retry can succeed, and, when a session ends while the portal is open, say so before returning the person to sign-in.

**Rationale**: A silent redirect mid-reading loses the reader's place and reads as a fault.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Unknown Addresses

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-not-found`

The system **MUST** tell a person who opens an address that names no view, or a view that no longer exists, that the view was not found, and offer a way to Home — never silently show a different view.

**Rationale**: A stale shared link that silently lands somewhere else is mistaken for the view it named.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-viewer`

### 5.2 Global View Controls

#### One Reporting Window Control

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-period-control`

The system **MUST** offer the same reporting-window control on every surface that shows period figures, with the presets Yesterday, Last 7 days, Last 30 days, Last 90 days, Last month, Last quarter and All time, the two most used reachable directly, every preset shown with the dates it resolves to, and a custom range; the active dates and their UTC basis are always visible. A preset applies on selection; a custom range applies on confirmation and refuses a range longer than the supported maximum, saying why.

**Rationale**: One control with one preset list removes the need to learn three, and visible dates remove doubt about what a preset means.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`

#### Window Semantics

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-period-semantics`

The system **MUST** resolve rolling presets as the stated number of UTC days up to and including today, Yesterday as the previous UTC day, Last month and Last quarter as the previous complete calendar month and quarter, and All time as all available history — and resolve each preset to the same dates wherever it is shown or computed.

**Rationale**: A preset whose dates a reader cannot predict cannot be compared with another report.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Window Scope per Surface

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-period-scope`

The system **MUST** keep one reporting window shared by Home, Explore, People and Reports, and a separate window per open dashboard that opens on the dashboard's own default; both are kept in the address, and changing one never changes the other. An address without a window opens on the person's last-used portal window, or Last 30 days for a first visit, and the opened window is then written into the address.

**Rationale**: A dashboard is authored for a window and shared by link; the portal-wide window is the reader's own working context.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-viewer`

#### View Controls Stay Reachable

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-pinned-toolbar`

The system **MUST** keep the reporting-window control visible while the content under it scrolls.

**Rationale**: The window applies to everything on screen; a reader scrolling a long view must still see and change it.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Org Scope from the Organisation

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-org-scope`

The system **MUST** let a lead or executive narrow people views to a part of the organisation they may see by choosing it from the work chart or a person's Team action, show which part is in effect on the views it affects, and offer a choice between direct reports only and the whole subtree where the chosen part has indirect reports.

**Rationale**: Scope is a choice about the organisation, made where the organisation is drawn.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`

#### Cohort

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-cohort`

The system **MUST** let a reader in a hierarchical organisation choose the cohort every comparison on screen is made against — the whole org scope, or the people sharing a person's value of an organisation attribute — explain what the choice controls, and offer no choice where only one cohort exists.

**Rationale**: "Versus median" is meaningless unless the reader knows, and can change, whom the median is over.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`

### 5.3 Home

#### Home Composition

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-home`

The system **MUST** show on Home, in order: starter dashboards for the viewer's connected sources; the three most recently changed dashboards the viewer owns, with a New Dashboard action for people who may author; per-person headline figures for the period with their change against the previous window; what needs attention, grouped by metric with the number of people affected; AI adoption; and a card per direction leading to it.

**Rationale**: Home answers "what should I look at" before "what are the numbers".

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-peer`

#### Home Pane

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-home-pane`

The system **MUST** list in Home's pane the overview themes — At a glance, By direction, Trend, Attention needed, Data coverage, Contribution breakdown — and a Dashboards group linking to My dashboards and Starter dashboards with their counts.

**Rationale**: The shortest path from landing to a saved dashboard starts on Home.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-peer`

### 5.4 Dashboards

#### Dashboards for Everyone

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-dashboards-access`

The system **MUST** open Dashboards to every signed-in person, showing each person only the dashboards, folders and pins the services return for them, and offering creating, editing, sharing, the catalogue, alerts and the Assistant only to administrators, as the sharing PRD revision 0.5 allows.

**Rationale**: Sharing is pointless if the people it is shared with cannot reach it.

**Actors**: `cpt-insightspec-portal-actor-viewer`, `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-admin`

#### Browse Filters

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-browse`

The system **MUST** let a person narrow the dashboard list to All, My dashboards, Shared with me, Starred or Starter, each with its count, from both the pane and a row of chips above the list, the two always showing the same selection.

**Rationale**: "Mine", "shared with me" and "starter" are the three questions a person brings to a list of dashboards.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Dashboard List

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-dashboard-list`

The system **MUST** list dashboards as cards showing the name; the owner where identity lets the person read the owner, and otherwise no owner; how long ago it changed; whether it is private, shared with people or shared with everyone, with the grants themselves named only to people who manage them; and its tags; with an open action and a menu of what the person may do with it; search by name; filter by tags; show the matching count; and load further pages as the person scrolls.

**Rationale**: Owner, age and visibility tell a person whether a dashboard is theirs, current and safe to share onward.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Starter Dashboards

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-starter`

The system **MUST** offer read-only starter dashboards for the viewer's connected sources, each stating the question it answers and the sources it reads with their connection state, open-only in the list, and savable as their own dashboard by people who may author.

**Rationale**: A new person needs a working dashboard before they know what to build.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Starred

- [ ] `p3` - **ID**: `cpt-insightspec-portal-fr-starred`

The system **MUST** let a person star and unstar any dashboard they can see, from its menu, and list their starred dashboards under Starred.

**Rationale**: A person reading many shared dashboards needs a short list of the ones they return to.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Folders, Tags and Pins

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-organise`

The system **MUST** let a person create, rename and delete folders, place a dashboard in folders, tag dashboards they may tag, and pin dashboards to the pane, as the services permit; show the folders and pinned dashboards in the pane; and say, before deleting a folder, what happens to the dashboards in it.

**Rationale**: Organising is what turns a list into a workspace.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Open Dashboard

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-open-dashboard`

The system **MUST** show an open dashboard's name; its owner and visibility as in `cpt-insightspec-portal-fr-dashboard-list`; the sources it reads with their connection state; when it last changed; its widgets in the stored order; its reporting window per `cpt-insightspec-portal-fr-period-scope`; an Edit action for people who may edit it; and a menu offering, as permitted, Duplicate, Share, Rename, Pin, Star, Move to folder, Tags and Delete, with Export snapshot and Schedule report shown as planned.

**Rationale**: A reader must know whose view this is, what it reads and how fresh it is before trusting it.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Widget Reading

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-widgets`

The system **MUST** draw each widget with its title, a subtitle naming its aggregation, grain and unit where it has them, the figure or chart, and, for people the services allow to drill down, a way to see the rows behind it; a headline figure shows its change against the previous window of the same length. A widget that fails shows a member a generic failure without its cause.

**Rationale**: A chart without its unit and grain, or a number without its comparison, cannot be read correctly.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-viewer`

#### Create Dashboard

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-create-dashboard`

The system **MUST** let a person who may author create a dashboard from a dialog reachable from Home and Dashboards that asks for a name, who can see it — Private by default, named people, or everyone — and a starting point — empty, a copy of a starter dashboard, or a description handed to the Assistant with the name and visibility already set — and then opens the new dashboard.

**Rationale**: One small step replaces a blank technical form, and describing a dashboard to the Assistant stays a first-class way to make one.

**Actors**: `cpt-insightspec-portal-actor-owner`

#### Share

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-share`

The system **MUST** let a person who may share a dashboard or folder grant it to named people or to everyone at the viewer or editor level, change a grant's level, list and withdraw grants, never preselect everyone, warn before an action — a grant, or placing a dashboard in a shared folder — widens who sees a dashboard, and offer Share only where the person may share.

**Rationale**: Sharing must be deliberate, visible and reversible.

**Actors**: `cpt-insightspec-portal-actor-owner`

#### Orphaned Dashboards and Folders

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-orphaned`

The system **MUST** list for administrators the dashboards and folders whose owner no longer holds the administrator role, with name, owner and last change, and let an administrator make a current administrator their owner.

**Rationale**: Work must stay in use when its owner changes role.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Catalogue

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-catalogue`

The system **MUST** list metrics, widgets, datasets and alerts in a Catalogue group under Dashboards, each list searchable and paged, each item opening a page that shows its definition, a run of it over a chosen window where it is dated, and what uses it.

**Rationale**: A dashboard is only as trustworthy as the definitions under it, and a person must find what a definition feeds before changing it.

**Actors**: `cpt-insightspec-portal-actor-admin`, `cpt-insightspec-portal-actor-owner`

#### Definition Editor

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-editor`

The system **MUST** let a person who may edit a definition change it through a form or as text, refuse invalid input field by field with the reason, never lose unsaved input on a refusal, and refuse to remove a definition while something still uses it, naming what does.

**Rationale**: Authoring is error-prone; a refusal that discards work or does not say why stops people authoring.

**Actors**: `cpt-insightspec-portal-actor-admin`, `cpt-insightspec-portal-actor-owner`

#### Alerts

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-alerts`

The system **MUST** let a person who may author define an alert over a metric — its condition, window, check interval, columns to include and destination — preview the metric's current value while defining it, turn it on and off, and see its latest check and notification history; and say when no destination is configured.

**Rationale**: An alert whose last check a person cannot see is not trusted.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Assistant

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-assistant`

The system **MUST** offer the Assistant beside every Dashboards surface to people who may author, without it taking the reader's screen until asked for, keeping the conversation while the person moves between Dashboards surfaces; each reply says whether it answered or built something, shows any result rows, and links what it built; a refused request shows the reason and returns the question to the input.

**Rationale**: Describing what one wants is how most dashboards are made; the Assistant must be at hand without taking the screen.

**Actors**: `cpt-insightspec-portal-actor-admin`, `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-assistant`

### 5.5 Explore

#### Directions and Lenses

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-lenses`

The system **MUST** list the directions with their lens counts and open each lens as a page of sections over the org scope and reporting window — headline figures with change, trends, distributions, concentration, composition and the other section kinds the lens defines — with the number of people the figures cover.

**Rationale**: A direction is read lens by lens; each lens must say how many people it is about.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-peer`

#### Evidence Behind a Figure

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-evidence`

The system **MUST** let a reader open, from any figure, band, trend point or cell, the people behind it where they hold the person-level grant and the records behind it where they hold the underlying-records grant ([rule 5](../../../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario)), drill from a person to their records in place, and export what is shown as CSV or Excel; without the grant the figure says the follow-back is not available to them.

**Rationale**: A figure that cannot be followed back is not acted on.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Repository Drill-Down

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-repository`

The system **MUST** open, from a repository row, that repository's figures — headline, contributors, activity by hour, composition by branch, ownership and event histogram — filtered to it, with a breadcrumb back to the lens.

**Rationale**: Delivery problems are often specific to one repository.

**Actors**: `cpt-insightspec-portal-actor-lead`

#### Organisation-Grain Lenses

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-org-grain`

The system **MUST** show lenses whose metrics belong to the organisation rather than to people, such as CI pipelines, without a roster, and say when the installation has not enabled those metrics.

**Rationale**: Some work has no person attached; forcing it into a per-person shape misleads.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### AI and Cost

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-ai-cost`

The system **MUST** show, to readers holding the cost grant, AI adoption and spend for the org scope, keeping seat-based and usage-based cost apart: potential cost, active users, accepted lines and average cost per active user; cost by tool; cost and usage per person; an adoption funnel; and a breakdown by the chosen cohort attribute — showing a reading the source does not provide as not tracked, never as zero.

**Rationale**: Finance and leads decide on AI spend from these figures; a missing reading shown as zero understates cost.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

### 5.6 People

#### Person Page

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-person`

The system **MUST** show for a person the viewer may see: their name, title and department; their manager where the manager is within the viewer's reach; which sections have data, which have none and which have no connected source; headline figures with trends against their cohort; what needs attention; and, per section, the figures and the work behind them.

**Rationale**: "What is visible about me" is the individual contributor's first question, and a lead's drill-down target.

**Actors**: `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`

#### Team View

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-team`

The system **MUST** show for the org scope its size, team-level summary figures, what needs attention, and every member in a grid compared within the cohort, ordered by name until the reader chooses another order, with a way to show a subset of members and to open any member's person page.

**Rationale**: A lead's period review starts with the team and ends with a person.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-peer`

#### Roster

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-roster`

The system **MUST** list every person the viewer may see with their title, organisational placement and status, searchable, grouped by role where its name says so, each opening the person page, and say when nobody matches.

**Rationale**: Finding a person is the most common way into their data.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-exec`

#### Work Chart

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-work-chart`

The system **MUST** draw, in the People pane, the reporting tree rooted at the viewer — or an alphabetical list in a flat organisation — searchable, opening a lead's team or a person's page on selection.

**Rationale**: The organisation's shape is how leads navigate their people.

**Actors**: `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-peer`

### 5.7 Reports

#### Report Builder

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-report-builder`

The system **MUST** let a reader pick metrics by family, choose people or organisation grain and a time granularity the reporting window supports, preview the resulting rows, and download them as CSV or Excel; say why the preview cannot run when it cannot; and let a running preparation be cancelled.

**Rationale**: Figures leave the product in spreadsheets; the export must be the same data the portal shows.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

### 5.8 Manage

#### Administration Destinations

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-manage`

The system **MUST** group administration under Data (Sources & connectors, Data exclusions, Org snapshots), People & access (Identities · roles & taxonomy, Access, Group management) and Platform (Ingestion, Previews, AI assistant config, What's new, Platform usage, MCP servers, Config & setup, Scorecard management), show each destination only to people who may use it, and open Manage on a working destination.

**Rationale**: Administrators need one place for the installation's health and configuration; landing on a placeholder wastes their first click.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Connector and Ingestion Health

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-ingestion-health`

The system **MUST** show each connector's state, last sync and records reported, its recent syncs, and the rate rows are arriving over recent and live windows per connector, distinguishing a connector never read from one read with nothing recorded.

**Rationale**: Every metric depends on ingestion; an administrator must see a stalled source before readers see wrong numbers.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Identity Resolution

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-identities`

The system **MUST** let an administrator work a review queue of unresolved accounts — merge, absorb and confirm — and look up a person's accounts or an account's owner.

**Rationale**: A person's figures are only as good as the accounts attributed to them.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Portal Usage

- [ ] `p3` - **ID**: `cpt-insightspec-portal-fr-usage`

The system **MUST** show administrators visits, people and pages opened over the reporting window, who opened what, which drill-downs and exports were used, and the feedback people sent.

**Rationale**: Adoption decides which surfaces to invest in.

**Actors**: `cpt-insightspec-portal-actor-admin`

### 5.9 States and Planned Surfaces

#### Distinct States

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-states`

The system **MUST** distinguish, on every surface and every independently loaded section, loading; no data for this reader; no connected source; withheld because the group is below the size a figure or comparison needs ([rule 10](../../../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario)), with the reason; a refusal with its reason; and a failure with a retry — and present each differently.

**Rationale**: "No data" and "could not load" lead to opposite conclusions.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-admin`

#### Viewing as Another Person

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-view-as`

The system **MUST** show, on every screen while an administrator views the portal as another person, whose view it is and a way to return to their own; and mark every screen drawn from demonstration data as such.

**Rationale**: An administrator acting under another person's reach, or a reader looking at demonstration figures, must never mistake them for their own.

**Actors**: `cpt-insightspec-portal-actor-admin`

#### Planned Surfaces

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-planned`

The system **MUST** hide planned surfaces unless the reader turns them on, mark them as planned where shown, never make a planned surface the default destination, and let an installation hide any surface or mark it planned.

**Rationale**: Signalling intent must not cost a reader a dead end, and each installation offers a different set of sources.

**Actors**: `cpt-insightspec-portal-actor-installation`, `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`

#### Destructive Actions

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-destructive`

The system **MUST** confirm every deletion and every action that widens who sees something before performing it, state its consequence in the confirmation, and present deletion as destructive.

**Rationale**: Deletions and widened sharing cannot be quietly undone.

**Actors**: `cpt-insightspec-portal-actor-owner`, `cpt-insightspec-portal-actor-admin`

### 5.10 Appearance and Access

#### Themes

- [ ] `p2` - **ID**: `cpt-insightspec-portal-fr-themes`

The system **MUST** render every surface in a light and a dark theme with the same meaning conveyed by colour in both, and follow the operating system's choice when the person picks system.

**Rationale**: Readers work in both; a status that disappears in one theme is a status lost.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

#### Keyboard and Screen Reader

- [ ] `p1` - **ID**: `cpt-insightspec-portal-fr-a11y`

The system **MUST** let every action be performed from the keyboard with visible focus, offer a skip to the main content, announce the current destination in both navigation levels, give every control an accessible name, and announce loading, refusals and errors.

**Rationale**: The portal is a workplace tool; an inaccessible one excludes colleagues.

**Actors**: `cpt-insightspec-portal-actor-exec`, `cpt-insightspec-portal-actor-lead`, `cpt-insightspec-portal-actor-ic`, `cpt-insightspec-portal-actor-peer`, `cpt-insightspec-portal-actor-admin`

## 6. Non-Functional Requirements

### 6.1 Quality Vector Analysis

| Quality Vector | Show-Stopper Requirement | Rationale |
|-----------------|---------------------------|-----------|
| Efficiency | None — no show-stopper; obligations covered by `cpt-insightspec-portal-nfr-bounded-requests` | Readers open lists of dozens of dashboards; requests that grow with the list slow every reader and load the services, but do not make the portal unusable. |
| Reliability | The system **MUST** never present a failed, refused or unconnected read as empty or zero (`cpt-insightspec-portal-nfr-honest-states`). | A lead who reads a failed load as "no activity" draws a wrong conclusion about named people; that destroys trust in every other figure. |
| Performance | None — no show-stopper; obligations covered by `cpt-insightspec-portal-nfr-feedback-latency` | Figures are computed by the services; the interface's own obligation is to acknowledge every action promptly so a wait is never mistaken for a fault. |
| Security | The system **MUST** never draw or export data, names or destinations outside the viewer's reach and grants (`cpt-insightspec-portal-nfr-reach`, `cpt-insightspec-portal-nfr-exports`). | Showing one person's figures to someone outside their reach is a privacy breach that ends the product's use in an organisation. |
| Versatility | None — no show-stopper; obligations covered by `cpt-insightspec-portal-nfr-accessibility`, `cpt-insightspec-portal-nfr-screen-widths`, `cpt-insightspec-portal-nfr-installation-policy` | Installations differ in sources, organisation shape and readers' devices and needs; each is covered, and none alone makes the portal unviable. |

### 6.2 NFR Inclusions

#### Honest States

- [ ] `p1` - **ID**: `cpt-insightspec-portal-nfr-honest-states`

**Vector**: Reliability

The system **MUST** present a failed, refused or not-connected read differently from an empty one, and a reading a source does not provide as not tracked rather than zero.

**Threshold**: Zero surfaces where a failed or refused read renders as an empty list, an empty chart or a zero.

**Rationale**: Conclusions about people are drawn from what the screen shows.

#### Reach Never Widened

- [ ] `p1` - **ID**: `cpt-insightspec-portal-nfr-reach`

**Vector**: Security

The system **MUST** show only what the services returned for the viewer, show a refusal rather than an empty view when the services refuse, and treat hiding a destination as a convenience, never as the control.

**Threshold**: No figure, name or destination is drawn that the services did not return for the signed-in person.

**Rationale**: The services enforce reach; the interface must not leak around them or suggest data exists where it is refused.

#### Bounded Requests per List

- [ ] `p2` - **ID**: `cpt-insightspec-portal-nfr-bounded-requests`

**Vector**: Efficiency

The system **MUST** show a page of any list — dashboards, catalogue items, people — with work that does not grow with the number of items on the page.

**Threshold**: The time and service work to show a list page are independent of how many items the page holds, up to the page size.

**Rationale**: A per-item request pattern makes the busiest lists the slowest and multiplies load on the services.

#### Prompt Feedback

- [ ] `p2` - **ID**: `cpt-insightspec-portal-nfr-feedback-latency`

**Vector**: Performance

The system **MUST** acknowledge every action visibly before any wait, and show a loading state for any wait longer than a moment.

**Threshold**: A loading state appears within 200 ms of any action whose result is not yet available. A page-load budget is an open decision owned by the product owner.

**Rationale**: An unacknowledged click is repeated or abandoned.

#### Accessibility

- [ ] `p1` - **ID**: `cpt-insightspec-portal-nfr-accessibility`

**Vector**: Versatility

The system **MUST** meet WCAG 2.2 level AA on every surface in both themes.

**Threshold**: WCAG 2.2 AA conformance, including text contrast of at least 4.5:1 and non-text contrast of at least 3:1.

**Rationale**: Colleagues with disabilities must be able to read and act on the same figures.

#### Screen Widths

- [ ] `p2` - **ID**: `cpt-insightspec-portal-nfr-screen-widths`

**Vector**: Versatility

The system **MUST** keep every surface usable from a 360 px-wide phone screen to a desktop screen without horizontal page scrolling.

**Threshold**: No horizontal page scroll at widths of 360 px and above; wide tables and charts scroll within their own frame.

**Rationale**: Leads read on phones between meetings.

#### Installation Policy

- [ ] `p2` - **ID**: `cpt-insightspec-portal-nfr-installation-policy`

**Vector**: Versatility

The system **MUST** let an installation hide or mark as planned any rail destination, pane destination, direction, lens, person-page section or metric through configuration, without rebuilding the portal.

**Threshold**: Every navigable surface and every metric can be hidden or marked planned by configuration alone.

**Rationale**: Installations connect different sources and roll surfaces out at different times.

#### Exports Carry Only What Is Shown

- [ ] `p1` - **ID**: `cpt-insightspec-portal-nfr-exports`

**Vector**: Security

The system **MUST** put into a CSV or Excel export only the rows the reader sees on screen under their grants, and record every export as a usage event naming the surface, never its contents.

**Threshold**: An export never contains a person, record or figure the reader could not see on screen; every export appears in portal usage.

**Rationale**: An exported file leaves the product's access rules; it must not take more than the reader already had.

### 6.3 NFR Exclusions

- Offline use: the portal shows live figures computed by the services; a cached offline view would present stale figures as current.
- Localisation: English is the only interface language in scope (§4.2).
- Availability and recovery: the portal is a static bundle with no state of its own; availability is inherited from the gateway and the services.
- Authentication and session lifetime: owned by the [authenticator PRD](../../backend/authenticator/PRD.md); the portal only presents session boundaries (`cpt-insightspec-portal-fr-session`).
- Personal-data retention, erasure and processing records: owned by the services that store the data; the portal stores no personal data beyond per-browser preferences.
- Scale targets — people per installation, dashboards per person: no portal-specific target; lists are paged and bounded by `cpt-insightspec-portal-nfr-bounded-requests`, and an installation-size target is an open decision owned by the product owner.
- Operator documentation: the portal has no operator-facing configuration beyond the installation policy, documented with the deployment chart.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Portal Addresses

- [ ] `p1` - **ID**: `cpt-insightspec-portal-interface-addresses`

**Type**: Data format — the portal's web addresses.

**Stability**: stable

**Description**: Every view, together with its reporting window, org scope, cohort, folder and tag filters, is named by its address, so an address can be bookmarked, shared and reopened.

**Breaking Change Policy**: An address that has been shared keeps opening the view it named, or a not-found view per `cpt-insightspec-portal-fr-not-found`; it never silently opens a different view.

### 7.2 External Integration Contracts

#### Insight Service Contracts

- [ ] `p1` - **ID**: `cpt-insightspec-portal-contract-services`

**Direction**: required from client

**Protocol/Format**: The analytics, identity, definition, sharing and assistant service interfaces, as specified by their own designs.

**Compatibility**: The portal is released with the services it calls; a refusal from a service is shown with its reason, never reinterpreted.

#### Installation Configuration Contract

- [ ] `p2` - **ID**: `cpt-insightspec-portal-contract-installation`

**Direction**: required from client

**Protocol/Format**: Per-installation configuration delivered to the browser at load, naming the surfaces to hide and to mark planned and the optional features enabled.

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
2. The Assistant opens with the name and visibility set; the person describes what they want.
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
- The person has read the dashboard without any editing, sharing or drill-down action offered.

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
- [ ] Every surface passes an automated WCAG 2.2 AA check in both themes, and a keyboard-only walkthrough reaches every action.
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
- [ ] Every surface at 360 px width shows no horizontal page scroll.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Analytics service | Computes every figure, trend, distribution and evidence list | p1 |
| Identity service | Decides each person's reach, roles and organisation placement | p1 |
| Definition service | Stores metrics, widgets, dashboards, datasets, alerts, folders, tags, pins | p1 |
| Sharing | Ownership, grants at viewer and editor levels to people or everyone, shared folders, orphaned transfer — [sharing PRD](../../../domain/insight-v3/specs/sharing/PRD.md) revision 0.5 | p1 |
| Window presets on the services | Every preset in `cpt-insightspec-portal-fr-period-control`, including Last 90 days, resolved per `cpt-insightspec-portal-fr-period-semantics` | p1 |
| Assistant service | Answers questions and builds definitions | p2 |
| Installation configuration | Hidden and planned surfaces, optional features | p2 |

## 11. Assumptions

- The services return each person's reach already applied; the portal never filters a broader result down.
- Starred dashboards and starter dashboards are provided by the services; the interface does not derive either.
- Which cohort attributes exist is decided by the services from the organisation's data.
- A WCAG 2.2 AA target is accepted by the product owner; a different target would be recorded as a revision of `cpt-insightspec-portal-nfr-accessibility`.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Starred and starter dashboards have no service support yet | Browse filters and Home sections show nothing to select | Show them as planned per `cpt-insightspec-portal-fr-planned` until the services provide them |
| Removing the top-bar scope control hides scope from leads who used it | Leads lose track of which part of the organisation a view covers | Every scoped view names its scope (`cpt-insightspec-portal-fr-org-scope`), and the work chart sets it |
| Placement of the cohort control is not settled by the visual design | The control moves again after release | The product owner decides placement before the cohort requirement is implemented |
| Existing dashboards carry a window restriction the single control ignores | A dashboard opens on a window its author did not intend | The dashboard's own default window opens first (`cpt-insightspec-portal-fr-period-scope`) |
| A metric refuses windows longer than its maximum | A widget on a dashboard read over a long preset shows a refusal | The widget shows the refusal with its reason per `cpt-insightspec-portal-fr-states` |
| The sharing PRD revision this PRD relies on is not yet merged | Requirements on grants, editors and members describe behaviour not yet available | Ship the dashboard requirements with the sharing release; until then show those actions as planned |
