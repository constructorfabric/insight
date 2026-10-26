---
status: draft
version: "0.4"
date: 2026-10-23
---

# PRD — Insight v3 Dashboard Ownership and Sharing

**Status:** [§4.1](#41-in-scope) approved for the first release. Revision 0.4 opens viewing to every signed-in person (D1, D8, D10–D12).

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
  - [5.1 Identity of Definitions](#51-identity-of-definitions)
  - [5.2 Ownership and Visibility](#52-ownership-and-visibility)
  - [5.3 Sharing](#53-sharing)
  - [5.4 Organising Dashboards](#54-organising-dashboards)
  - [5.5 Ownerless Dashboards](#55-ownerless-dashboards)
  - [5.6 Assistant and MCP](#56-assistant-and-mcp)
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

Every custom dashboard gets an owner and stays private until the owner shares it, view-only, with named people. This expands the parent's [§5.4](../PRD.md#54-metrics-widgets-and-dashboards).

### 1.2 Background / Problem Statement

Today every administrator sees and changes every custom dashboard, nothing records its author, and folders and tags are one shared set. Dashboards are keyed by a name unique across the installation, so a private one would still reserve its name, and refusing that name would reveal it.

### 1.3 Goals (Business Outcomes)

- Work stays private until shared, then reaches only chosen colleagues.
- Nothing is lost when an owner stops being an administrator, and existing dashboards work as now.
- Two people can use the same name.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Definition | A stored metric, widget, dashboard or dataset |
| Ownerless | A dashboard, folder or tag stored before ownership existed |
| Orphaned | A dashboard whose owner no longer holds the administrator role |
| Member | A signed-in person without the administrator role |

## 2. Actors

### 2.1 Human Actors

#### Owner

**ID**: `cpt-insightspec-v3-sharing-actor-owner`

**Role**: The administrator who created or duplicated a dashboard, or received it by transfer.

**Needs**: Privacy until they share; who sees each dashboard.

#### Viewer

**ID**: `cpt-insightspec-v3-sharing-actor-viewer`

**Role**: Any signed-in person a dashboard is shared with, administrator or member.

**Needs**: Shared dashboards easy to find; nothing in their way they cannot use.

#### Administrator

**ID**: `cpt-insightspec-v3-sharing-actor-admin`

**Role**: Any authenticated Insight v3 administrator; the only people who author.

**Needs**: Ownerless dashboards unchanged; orphaned ones kept in use.

#### Operator

**ID**: `cpt-insightspec-v3-sharing-actor-operator`

**Role**: Runs the installation; assigns owners to ownerless dashboards in the database.

**Needs**: A lossless upgrade.

### 2.2 System Actors

#### AI Assistant

**ID**: `cpt-insightspec-v3-sharing-actor-assistant`

**Role**: The portal chat and the MCP server, acting for the signed-in person.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

One installation holds one tenant's definitions. People and the administrator role come from the identity service; when identity stops granting a person the role, their dashboards become orphaned. Every signed-in person reaches the Custom zone; the role decides what they may do there.

## 4. Scope

### 4.1 In Scope

- Generated identities, with a lossless upgrade.
- Owners, view-only shares with any signed-in person, transfer, personal folders, owner's tags, personal pins, and ownerless pre-release rows.
- Viewing by members: a shared dashboard drawn through the dashboard alone, with no reach into the metric and widget library.
- The Custom zone, API, chat and MCP, acting as the signed-in person.

### 4.2 Out of Scope

- Authoring, the library, chat, MCP, alerts, orphaned and ownerless dashboards for members; member authoring over the shared library is the next step (D1).
- Per-viewer data scoping: a viewer sees what the dashboard's metrics return (D10).
- Drilldown for members (D11).
- Edit access for anyone but the owner; owners for metrics, widgets and datasets.
- Public dashboards, links, and sharing with a group, role, organisation unit or attribute.
- Handing over a dashboard while its owner is still an administrator; claiming an ownerless one in the product.
- Share notifications beyond the shared list; sharing, withdrawing or transferring through the chat or MCP.

## 5. Functional Requirements

### 5.1 Identity of Definitions

#### Address Definitions by Identity

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-identity`

Every metric, widget, dashboard, folder and tag **MUST** have an identity, used by every operation and every stored reference, alerts and portal addresses included; a new one gets a generated identity. Names are labels: a rename rewrites no reference. A tag is still applied by name, resolved to the dashboard owner's tag of that name and created if absent (`cpt-insightspec-v3-sharing-fr-tags`). A dataset's unique name stays its identity (D6). Only tag and dataset names are unique.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-admin`, `cpt-insightspec-v3-sharing-actor-assistant`

#### Keep Existing Definitions

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-upgrade`

The upgrade **MUST** keep every existing definition, folder, tag, pin, folder placement and alert, with each existing name becoming that definition's identity, so every stored reference and every link stays valid without being rewritten.

**Actors**: `cpt-insightspec-v3-sharing-actor-operator`

### 5.2 Ownership and Visibility

#### Own Dashboards

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-owner`

A dashboard's creator, on any surface, **MUST** own it; nothing creates an unowned one. A duplicate is its maker's, with the original's content, its tag names as the maker's tags, and the maker's filing of it if in the maker's own folder.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-assistant`

#### See Only What Is Yours or Shared With You

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-visibility`

A person **MUST** see a dashboard only if they own it, it is shared with them, or it is ownerless and they are an administrator (`cpt-insightspec-v3-sharing-nfr-isolation`); the administrator role grants nothing more, and a member never sees an ownerless one. Owning needs the role: a person who loses it sees their own dashboards only once one is shared back or transferred to them. A member sees a dashboard's owner by name only where identity lets them read that person; otherwise no owner is shown. Lists show owners and narrow to owned or shared-with-me.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-viewer`, `cpt-insightspec-v3-sharing-actor-admin`

#### Restrict Changes to the Owner

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-owner-actions`

The system **MUST** let only the owner edit, rename, delete, duplicate, tag and share a dashboard; for an ownerless one, `cpt-insightspec-v3-sharing-fr-ownerless` applies. A viewer only opens, pins and files it, so a withdrawn share leaves them nothing; anything else is refused as not permitted.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-viewer`

#### Draw Through the Dashboard

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-render`

A viewer **MUST** read a dashboard's widgets and run their metrics only through a dashboard they can see, and only for widgets that dashboard holds; a widget it does not hold answers as missing. A viewer may ask the widget's metric for any time window, as the page itself does. A member gets no drilldown. A widget that fails answers a member with a generic message, the cause going to the log.

**Actors**: `cpt-insightspec-v3-sharing-actor-viewer`, `cpt-insightspec-v3-sharing-actor-admin`

#### Keep the Library Shared

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-fr-library`

Metrics, widgets and datasets **MUST** stay visible to and editable by administrators alone. Reports of what uses a widget or metric, a refused deletion included, **MUST** name only visible dashboards and count the rest. Deleting something in use stays refused, whoever's dashboards use it.

**Actors**: `cpt-insightspec-v3-sharing-actor-admin`

### 5.3 Sharing

#### Share With Named People

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-share`

An owner **MUST** be able to share a dashboard for viewing with one person picked by name and bound by identity's person id, and list its shares. Any person identity knows is offered and accepted; the administrator role is not required, and there is no cap. Sharing twice changes nothing. Viewers see the owner, not each other.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`

#### Withdraw a Share

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-unshare`

An owner **MUST** be able to withdraw a share; from that person's next request the dashboard, their pin and their filing of it are gone.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`

#### Transfer an Orphaned Dashboard

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-transfer`

Every administrator **MUST** see the orphaned dashboards, with only name, owner and last change time, and be able to make a current administrator the owner of one; its shares stay. A dashboard whose owner still holds the role is neither listed nor transferable.

**Actors**: `cpt-insightspec-v3-sharing-actor-admin`

### 5.4 Organising Dashboards

#### Personal Folders

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-folders`

Folders created from this release on **MUST** be seen and changed by their creator alone. A person files a dashboard they own or that is shared with them in at most one of their folders, invisibly to others; deleting the folder unfiles it. Folder counts cover only visible dashboards; folders are capped per person; neither reveals another person's folders.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-viewer`

#### Owner's Tags

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-tags`

Only a dashboard's owner **MUST** set its tags, from their own, named uniquely per owner ignoring case. A person sees only tags on visible dashboards, counted over those; filtering by tag name matches every visible dashboard with a tag of that name. Tags are capped per owner and vanish when no dashboard carries them.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-viewer`

#### Personal Pins

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-fr-pins`

Pins **MUST** stay personal and reach only dashboards the person may see.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-viewer`

### 5.5 Ownerless Dashboards

#### Keep Ownerless Dashboards as They Are

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-ownerless`

Every dashboard, folder and tag stored before this release **MUST** stay ownerless, seen, edited and deleted by every administrator as now, and by no member. Ownerless dashboards file only in ownerless folders, which hold nothing else, and carry ownerless tags, which any administrator sets and a new name creates. They cannot be shared or transferred. The product **MUST NOT** create ownerless dashboards or folders, or make anything owned ownerless; only the operator assigns owners, in the database.

**Actors**: `cpt-insightspec-v3-sharing-actor-admin`, `cpt-insightspec-v3-sharing-actor-operator`

#### Retire Ownerless Dashboards

- [ ] `p3` - **ID**: `cpt-insightspec-v3-sharing-fr-retire-ownerless`

The operator procedure for giving an ownerless dashboard an owner **MUST** be documented, ending as if that person created it: out of its ownerless folder, its tags replaced by the owner's same-named tags. A later upgrade step **MUST** make owners required on dashboards, folders and tags, refusing with counts while any ownerless dashboard, folder or tag, or owned dashboard in an ownerless folder or with an ownerless tag, remains.

**Actors**: `cpt-insightspec-v3-sharing-actor-operator`

### 5.6 Assistant and MCP

#### Act as the Signed-In Person

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-assistant`

The chat and MCP **MUST** act as the signed-in person: same sight and rights, creations owned by that person, and a dashboard replaced only if that person owns it or it is ownerless. They create by returning a new identity and change by naming one; a name never means replace. Both stay with administrators (D1).

**Actors**: `cpt-insightspec-v3-sharing-actor-assistant`

## 6. Non-Functional Requirements

### 6.1 Quality Vector Analysis

| Quality Vector | Show-Stopper Requirement | Rationale |
|----------------|-------------------------|-----------|
| Efficiency | None — no show-stopper; obligations covered by `cpt-insightspec-v3-nfr-efficiency` | A few small tables and checks fit the parent footprint |
| Reliability | The upgrade MUST lose nothing, `cpt-insightspec-v3-sharing-nfr-upgrade-integrity` | A dashboard drawing nothing is lost work nobody notices |
| Performance | None — no show-stopper; obligations covered by `cpt-insightspec-v3-sharing-nfr-list-latency` and `cpt-insightspec-v3-nfr-performance` | Visibility filtering must not slow lists |
| Security | A dashboard MUST reach only its owner, its viewers and, while ownerless, every administrator, `cpt-insightspec-v3-sharing-nfr-isolation`; a member MUST reach nothing beyond their dashboards, `cpt-insightspec-v3-sharing-nfr-reach`; access changes MUST be traceable, `cpt-insightspec-v3-sharing-nfr-audit` | Privacy is the feature's one promise, and sharing a page must not hand over the library behind it |
| Versatility | None — no show-stopper; obligations covered by `cpt-insightspec-v3-nfr-versatility` | One rule serves every surface |

### 6.2 NFR Inclusions

#### Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-nfr-isolation`

**Vector**: Security

Nothing about an invisible dashboard (existence, name, owner, tags, folder, content) **MUST** reach a person, except the hidden count of `cpt-insightspec-v3-sharing-fr-library` and the orphaned list of `cpt-insightspec-v3-sharing-fr-transfer`.

**Threshold**: beyond those, zero disclosure in lists, totals, searches, reads, refusals, pins, folders, tags, counts, caps, dependents, usage reports, chat context and MCP tools, to members and administrators alike. An invisible dashboard answers exactly like a missing one. A viewer's refused change reveals nothing new.

**Rationale**: The feature is only as good as its privacy.

#### Bounded Reach

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-nfr-reach`

**Vector**: Security

A member **MUST** reach nothing in the Custom zone but the dashboards shared with them, what those dashboards draw, and their own pins and folders.

**Threshold**: every library, creation, direct metric-run, drilldown, chat, MCP, orphaned, alert, dataset and ownerless surface refuses a member as not permitted; a widget drawn through a dashboard, the one way a member runs a metric, is one that dashboard holds; no message to a member names a warehouse table or column, or a dataset. A member does read the widgets a shared dashboard holds, which name their metric and detail metric and the result columns they draw.

**Rationale**: A shared page is a window onto chosen data, not a key to the warehouse.

#### Traceable Access Changes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-nfr-audit`

**Vector**: Security

Every share, withdrawal and transfer **MUST** be recorded.

**Threshold**: each is logged with actor, dashboard, person concerned and time; a share stores who granted it.

**Rationale**: Who can see what must be explainable afterwards.

#### Upgrade Integrity

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-nfr-upgrade-integrity`

**Vector**: Reliability

The move to identities **MUST** finish intact, also when re-run after an interruption, while the previous release keeps serving; that release's writes **MUST NOT** land unconverted or duplicated.

**Threshold**:

- Counts of definitions, pins, placements, tags and alerts match before and after.
- Every reference that resolved by name resolves by identity: dashboards draw the same widgets in the same order, alerts watch the same metric.
- A run stopped at any step and restarted ends in the same state.
- A previous-release write after the keys change is refused, not stored.

**Rationale**: The upgrade rewrites every stored reference at once.

#### List Latency

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-nfr-list-latency`

**Vector**: Performance

Listing and opening dashboards **MUST** meet the parent's targets with visibility applied.

**Threshold**: open p95 under 2 s at reference-organization scale; listing p95 no worse than the previous release's at that scale; a listing reads one page of visible dashboards, not every dashboard.

**Rationale**: Privacy must not cost waiting time.

#### Parent Obligations

**Inherits**: `cpt-insightspec-v3-nfr-efficiency`, `cpt-insightspec-v3-nfr-reliability`, `cpt-insightspec-v3-nfr-performance`, `cpt-insightspec-v3-nfr-security`, `cpt-insightspec-v3-nfr-versatility`.

Targets unchanged; evidence pending.

### 6.3 NFR Exclusions

None.

## 7. Public Library Interfaces

### 7.1 Public API Surface

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-interface-definitions`

**Type**: API and MCP tools.

**Stability**: Proposed, unreleased.

**Description**: Operations take identities; API and MCP share one visibility rule; dashboard-scoped reads serve viewers.

**Breaking Change Policy**: Routes and tools move from names to identities together with the portal, chat and MCP in one release; routes by name are removed. See [DESIGN](./DESIGN.md).

### 7.2 External Integration Contracts

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-contract-identity`

**Direction**: Outbound to the identity service.

**Protocol/Format**: The caller, their administrator role, current administrators, whether a person exists, person ids, display names.

**Compatibility**: Only what identity already offers administrators.

## 8. Use Cases

#### Share a Dashboard With a Colleague

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-usecase-share`

**Actor**: `cpt-insightspec-v3-sharing-actor-owner`

**Preconditions**: The colleague is a signed-in person, administrator or not.

**Main Flow**:

1. The owner shares the dashboard; the colleague finds, opens, pins and files it, and its widgets draw.
2. The colleague cannot open the metrics or widgets behind it on their own.
3. The owner withdraws the share; it leaves the colleague's list, pins and folders.

**Postconditions**: Only the owner and its viewers saw it.

**Alternative Flows**: A person identity does not know is not offered.

#### Keep an Orphaned Dashboard in Use

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-usecase-transfer`

**Actor**: `cpt-insightspec-v3-sharing-actor-admin`

**Preconditions**: The owner no longer holds the administrator role.

**Main Flow**:

1. An administrator finds the dashboard in the orphaned list.
2. They transfer it to an administrator, possibly themselves.

**Postconditions**: An administrator owns it; viewers keep it.

## 9. Acceptance Criteria

- [ ] Tests prove every requirement in §5 and §6.2 on each surface it names, `cpt-insightspec-v3-sharing-nfr-isolation` as an administrator and as a member who is neither owner nor viewer.
- [ ] `cpt-insightspec-v3-sharing-fr-render` and `cpt-insightspec-v3-sharing-nfr-reach`: a member opens a shared dashboard and its widgets draw; the same member is refused on every library, creation, direct metric-run, drilldown, chat, MCP, orphaned, alert, dataset and ownerless surface; a widget the dashboard does not hold answers as missing; a failing widget tells the member nothing of its warehouse table or columns, or its dataset.
- [ ] `cpt-insightspec-v3-sharing-fr-owner`: a duplicate of an ownerless dashboard carries its tag names as the duplicator's and sits in no folder.
- [ ] `cpt-insightspec-v3-sharing-fr-identity`: same-named dashboards, folders, widgets or metrics coexist; same-named tags only under different owners; same-named datasets never.
- [ ] Parent quality gates hold, including coverage and non-degradation.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Insight v3 definitions | Definitions, folders, tags, pins and their references | p1 |
| Identity service | Caller, administrator role, current administrators, whether a person exists, display names | p1 |
| Custom zone | Open to every signed-in person; the role decides what is offered | p1 |
| Metric alerts | Refer to metrics; move to identities with them | p2 |

## 11. Assumptions

Decisions taken by the product owner:

- **D1.** Administrators author; any signed-in person may be a viewer. Member authoring over the shared library is the next step.
- **D2.** Shares are view-only.
- **D3.** The administrator role shows no private dashboard.
- **D4.** Orphaned means the owner lost the role, which covers a leaver when leaving ends it.
- **D5.** Ownerless rows stay; the operator, not a claim feature, assigns owners.
- **D6.** Tag names are unique per owner, since tags are applied by name and created on first use; a dataset's unique name stays its identity, since outside producers address datasets by name, and unowned datasets disclose nothing. Existing definitions keep their former name as their identity, so nothing stored is rewritten.
- **D7.** Folders are places; tags group a dashboard several ways.
- **D8.** Share targets are any person identity knows; the sharer is always an administrator.
- **D9.** No sharing or transfer through the chat or MCP.
- **D10.** A viewer sees what the dashboard's metrics return, over any time window the page can ask for, with no per-viewer scoping; sharing is the owner vouching for that person.
- **D11.** Members get no drilldown: a detail metric returns rows the dashboard does not show.
- **D12.** A member's errors are generic; the cause goes to the log.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Searching a body by the name of a metric or widget it uses | No match | Search the definition's own name; dependents show its users |
| Identity unreachable | Nobody identified | Server error, never allowed, like the administrator check now |
| An owner loses the role briefly | Dashboard transferred meanwhile | Accepted; the new owner can share it back |
| A viewer sees data the rest of the portal would scope away from them | Disclosure beyond the viewer's own team | D10: the owner chooses who sees a dashboard; per-viewer scoping is its own feature |
| A member probes the library through dashboard-scoped runs | Reach beyond the shared page | `cpt-insightspec-v3-sharing-nfr-reach`: a run names a widget the dashboard holds or answers as missing |
