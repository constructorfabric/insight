---
status: draft
version: "0.5"
date: 2026-10-26
---

# PRD — Insight v3 Dashboard Ownership and Sharing

**Status:** [§4.1](#41-in-scope) approved for the first release. Revision 0.5 adds editors who share onward, sharing with everyone, and folders shared as containers that carry their audience to the dashboards placed in them (D13–D20).

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

Every custom dashboard and folder gets an owner and stays private until it is shared, for viewing or editing, with named people or with everyone. A shared folder reaches the dashboards placed in it. This expands the parent's [§5.4](../PRD.md#54-metrics-widgets-and-dashboards).

### 1.2 Background / Problem Statement

Today every administrator sees and changes every custom dashboard, nothing records its author, and folders and tags are one shared set. Dashboards are keyed by a name unique across the installation, so a private one would still reserve its name, and refusing that name would reveal it. Teams that work on the same set of dashboards need to edit them together and to share the set once rather than each dashboard on its own.

### 1.3 Goals (Business Outcomes)

- Work stays private until shared, then reaches only chosen colleagues or, by an explicit choice, everyone.
- Colleagues edit a dashboard together, and a team shares a folder once for every dashboard in it.
- Nothing is lost when an owner stops being an administrator, and existing dashboards work as now.
- Two people can use the same name.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Definition | A stored metric, widget, dashboard or dataset |
| Ownerless | A dashboard, folder or tag stored before ownership existed |
| Member | A signed-in person without the administrator role |
| Grant | Access to one dashboard or folder given to one person or to everyone, at the viewer or editor level |
| Everyone | Every signed-in person, as the recipient of a grant |
| Editor | The level of a grant that lets an administrator change a dashboard or folder and share it onward |
| Share onward | Managing a dashboard's or folder's grants as its editor |
| Entry | One dashboard's place in one folder; a dashboard may have entries in many folders |
| Carrying entry | An entry whose placer owns the dashboard or holds an editor grant on it, directly or through everyone; it gives the folder's audience access to the dashboard |
| Filing entry | Any other entry; it gives nobody access |
| Orphaned dashboard | A dashboard whose owner no longer holds the administrator role |
| Orphaned folder | A folder with at least one grant whose owner does not hold the administrator role |

## 2. Actors

### 2.1 Human Actors

#### Owner

**ID**: `cpt-insightspec-v3-sharing-actor-owner`

**Role**: The administrator who created or duplicated a dashboard, or received it by transfer; for a folder, any signed-in person who created it or received it by transfer.

**Needs**: Privacy until they share; control over who sees and who edits.

#### Editor

**ID**: `cpt-insightspec-v3-sharing-actor-editor`

**Role**: An administrator granted the editor level on a dashboard or folder, directly, through everyone, or through a folder carrying the dashboard.

**Needs**: Change and share what they were trusted with, without asking the owner.

#### Viewer

**ID**: `cpt-insightspec-v3-sharing-actor-viewer`

**Role**: Any signed-in person who sees a dashboard or folder without owning or editing it, administrator or member.

**Needs**: Shared dashboards easy to find and arrange in their own folders; nothing in their way they cannot use.

#### Administrator

**ID**: `cpt-insightspec-v3-sharing-actor-admin`

**Role**: Any authenticated Insight v3 administrator; the only people who author and share.

**Needs**: Ownerless dashboards and folders unchanged; orphaned ones kept in use.

#### Operator

**ID**: `cpt-insightspec-v3-sharing-actor-operator`

**Role**: Runs the installation; assigns owners to ownerless dashboards and folders in the database.

**Needs**: A lossless upgrade.

### 2.2 System Actors

#### AI Assistant

**ID**: `cpt-insightspec-v3-sharing-actor-assistant`

**Role**: The portal chat and the MCP server, acting for the signed-in person.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

One installation holds one tenant's definitions. People and the administrator role come from the identity service; when identity stops granting a person the role, their dashboards, and their folders that carry a grant, become orphaned. Every signed-in person reaches the Custom zone; the role decides what they may do there.

## 4. Scope

### 4.1 In Scope

- Generated identities, with a lossless upgrade.
- Owners of dashboards and folders; grants at the viewer or editor level to any signed-in person or to everyone; sharing onward by editors; transfer of orphaned dashboards and folders.
- Folders as shared containers holding dashboards through carrying and filing entries; personal folders for every signed-in person; owner's tags; personal pins; ownerless pre-release rows.
- Viewing by members: a dashboard drawn through the dashboard alone, with no reach into the metric and widget library.
- The Custom zone, API, chat and MCP, acting as the signed-in person.

### 4.2 Out of Scope

- Authoring, editing, sharing, the library, chat, MCP, alerts, orphaned and ownerless dashboards for members; member authoring over the shared library is the next step (D1).
- Per-viewer data scoping: a viewer sees what the dashboard's metrics return (D10).
- Drilldown for members (D11).
- Owners for metrics, widgets and datasets.
- Links, and sharing with a group, role, organisation unit or attribute.
- Folders inside folders.
- Handing over a dashboard or folder while its owner is still an administrator; claiming an ownerless one in the product.
- Share notifications beyond the shared list; sharing, withdrawing or transferring through the chat or MCP.

## 5. Functional Requirements

### 5.1 Identity of Definitions

#### Address Definitions by Identity

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-identity`

Every metric, widget, dashboard, folder and tag **MUST** have an identity, used by every operation and every stored reference, alerts and portal addresses included; a new one gets a generated identity. Names are labels: a rename rewrites no reference. A tag is still applied by name, resolved to the dashboard owner's tag of that name and created if absent (`cpt-insightspec-v3-sharing-fr-tags`). A dataset's unique name stays its identity (D6). Only tag and dataset names are unique.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-admin`, `cpt-insightspec-v3-sharing-actor-assistant`

#### Keep Existing Definitions

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-upgrade`

The upgrade **MUST** keep every existing definition, folder, tag, pin, folder placement and alert, with each existing name becoming that definition's identity, so every stored reference and every link stays valid without being rewritten. A dashboard filed in a folder stays in that folder.

**Actors**: `cpt-insightspec-v3-sharing-actor-operator`

### 5.2 Ownership and Visibility

#### Own Dashboards and Folders

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-owner`

A dashboard's creator, on any surface, **MUST** own it, and so **MUST** a folder's; nothing creates an unowned one. A duplicate is its maker's, with the original's content, its tag names as the maker's tags, and a carrying entry in each of the original's owned folders the maker can edit.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-editor`, `cpt-insightspec-v3-sharing-actor-assistant`

#### See Only What Is Yours or Shared With You

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-visibility`

A person **MUST** see a dashboard only if they own it, it is granted to them or to everyone, it has a carrying entry in a folder they see, or it is ownerless and they are an administrator; they **MUST** see a folder only if they own it, it is granted to them or to everyone, or it is ownerless and they are an administrator (`cpt-insightspec-v3-sharing-nfr-isolation`). The administrator role grants nothing more, and a member never sees an ownerless dashboard or folder. Through a carrying entry, a folder's owners and editors edit the dashboard and its viewers view it. The strongest of these applies, then `cpt-insightspec-v3-sharing-fr-editor-cap`. Owning a dashboard needs the role: a person who loses it reaches their own dashboards only through grants or folders, until a transfer. A member sees a dashboard's owner by name only where identity lets them read that person; otherwise no owner is shown. Lists show owners and narrow to owned or shared-with-me.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-editor`, `cpt-insightspec-v3-sharing-actor-viewer`, `cpt-insightspec-v3-sharing-actor-admin`

#### Cap Editing by Role

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-editor-cap`

The editor level **MUST** take effect only for an administrator: a member granted it, directly, through everyone or through a folder, only views, everywhere, and edits as soon as they hold the role. A folder's owner keeps owning it whatever their role.

**Actors**: `cpt-insightspec-v3-sharing-actor-editor`, `cpt-insightspec-v3-sharing-actor-viewer`

#### Restrict Changes to Owners and Editors

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-owner-actions`

On a dashboard, the system **MUST** let:

- its owner and editors edit, rename, arrange, tag, duplicate and share it, and place it in folders, carrying as `cpt-insightspec-v3-sharing-fr-entries` decides;
- only its owner delete it;
- its viewers only open it, pin it and place it in folders with filing entries.

On a folder, its owner and editors rename it and add and remove its entries, those of them who are administrators share it, and only its owner deletes it; its viewers only list it. Ownerless rows follow `cpt-insightspec-v3-sharing-fr-ownerless`; transfer follows `cpt-insightspec-v3-sharing-fr-transfer`. Anything else is refused as not permitted, naming who may do it.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-editor`, `cpt-insightspec-v3-sharing-actor-viewer`

#### Draw Through the Dashboard

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-render`

Anyone who sees a dashboard **MUST** read its widgets and run their metrics through that dashboard, and only for widgets it holds; a widget it does not hold answers as missing. They may ask the widget's metric for any time window, as the page itself does. A member gets no drilldown. A widget that fails answers a member with a generic message, the cause going to the log.

**Actors**: `cpt-insightspec-v3-sharing-actor-viewer`, `cpt-insightspec-v3-sharing-actor-admin`

#### Keep the Library Shared

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-fr-library`

Metrics, widgets and datasets **MUST** stay visible to and editable by administrators alone. Reports of what uses a widget or metric, a refused deletion included, **MUST** name only visible dashboards and count the rest. Deleting something in use stays refused, whoever's dashboards use it.

**Actors**: `cpt-insightspec-v3-sharing-actor-admin`

### 5.3 Sharing

#### Share With People or With Everyone

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-share`

An administrator who owns or edits a dashboard or folder **MUST** be able to grant it at the viewer or editor level to one person, picked by name and bound by identity's person id, or to everyone, and list its grants. Any person identity knows is offered and accepted; the administrator role is not required, and there is no cap. Granting to everyone is a separate, explicit choice, never preselected. Granting again sets the level; nothing else changes. Viewers see the owner, not each other. An ownerless dashboard or folder cannot be shared.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-editor`

#### Share Onward

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-share-onward`

An editor **MUST** manage grants as the owner does: add one, change its level, and withdraw any, everyone's and other editors' included. The owner's own access is not a grant and **MUST NOT** be withdrawable.

**Actors**: `cpt-insightspec-v3-sharing-actor-editor`, `cpt-insightspec-v3-sharing-actor-owner`

#### Withdraw a Grant

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-unshare`

An owner or editor **MUST** be able to withdraw a grant; from that person's next request they lose what it gave, unless another grant, everyone or a folder still gives access. Their pins and entries **MUST** stay, shown only while they can see the dashboard, so they return if access returns; a carrying entry they placed files from then on (`cpt-insightspec-v3-sharing-fr-entries`).

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-editor`

#### Transfer an Orphaned Dashboard or Folder

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-transfer`

Every administrator **MUST** see the orphaned dashboards, with only name, owner and last change time, and the orphaned folders, with only name and owner, and be able to make a current administrator the owner of one; its grants and entries stay, the previous owner's carrying entries filing from then on (`cpt-insightspec-v3-sharing-fr-entries`). One whose owner still holds the role, or a folder with no grant, is neither listed nor transferable.

**Actors**: `cpt-insightspec-v3-sharing-actor-admin`

### 5.4 Organising Dashboards

#### Folders as Shared Containers

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-folders`

Any signed-in person **MUST** be able to create folders, one level deep and capped per owner; a folder without grants is personal. A folder holds dashboards through entries, and a dashboard may have entries in any number of folders. Removing an entry or deleting a folder never deletes a dashboard. A folder's listing and counts cover only the dashboards visible to the person reading it; neither its contents nor the cap reveals another person's folders.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-editor`, `cpt-insightspec-v3-sharing-actor-viewer`

#### Carrying and Filing Entries

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-entries`

An entry **MUST** carry the folder's audience to the dashboard (`cpt-insightspec-v3-sharing-fr-visibility`) only while its placer, an administrator when placing it, owns the dashboard or holds an editor grant on it, directly or through everyone; editing that comes through a folder does not qualify. Every other entry **MUST** carry nothing: the dashboard shows in that folder only to people who see it anyway. Whenever a dashboard's grants or owner change so that a placer no longer qualifies, their entries for it **MUST** stop carrying at once; a placer losing the administrator role is not followed. Before a carrying entry goes into a shared folder, or a folder holding carrying entries is shared, the person **MUST** be told who gains access.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-editor`, `cpt-insightspec-v3-sharing-actor-viewer`

#### Owner's Tags

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-tags`

Only a dashboard's owner and editors **MUST** set its tags, always from the owner's tags, named uniquely per owner ignoring case. A person sees only tags on visible dashboards, counted over those; filtering by tag name matches every visible dashboard with a tag of that name. Tags are capped per owner and vanish when no dashboard carries them.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-editor`, `cpt-insightspec-v3-sharing-actor-viewer`

#### Personal Pins

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-fr-pins`

Pins **MUST** stay personal, be made only on dashboards the person may see, and show only those they may see now.

**Actors**: `cpt-insightspec-v3-sharing-actor-owner`, `cpt-insightspec-v3-sharing-actor-viewer`

### 5.5 Ownerless Dashboards

#### Keep Ownerless Dashboards as They Are

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-ownerless`

Every dashboard, folder and tag stored before this release **MUST** stay ownerless, seen, edited and deleted by every administrator as now, and by no member. Ownerless dashboards sit only in ownerless folders, which hold nothing else, and carry ownerless tags, which any administrator sets and a new name creates. They cannot be shared or transferred. The product **MUST NOT** create ownerless dashboards or folders, or make anything owned ownerless; only the operator assigns owners, in the database.

**Actors**: `cpt-insightspec-v3-sharing-actor-admin`, `cpt-insightspec-v3-sharing-actor-operator`

#### Retire Ownerless Dashboards

- [ ] `p3` - **ID**: `cpt-insightspec-v3-sharing-fr-retire-ownerless`

The operator procedure for giving an ownerless dashboard an owner **MUST** be documented, ending as if that person created it: out of its ownerless folders, its tags replaced by the owner's same-named tags. A later upgrade step **MUST** make owners required on dashboards, folders and tags, refusing with counts while any ownerless dashboard, folder or tag, or owned dashboard in an ownerless folder or with an ownerless tag, remains.

**Actors**: `cpt-insightspec-v3-sharing-actor-operator`

### 5.6 Assistant and MCP

#### Act as the Signed-In Person

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-fr-assistant`

The chat and MCP **MUST** act as the signed-in person: same sight and rights, creations owned by that person, and a dashboard replaced only if that person owns or edits it or it is ownerless. They create by returning a new identity and change by naming one; a name never means replace. Both stay with administrators (D1) and neither shares nor transfers (D9).

**Actors**: `cpt-insightspec-v3-sharing-actor-assistant`

## 6. Non-Functional Requirements

### 6.1 Quality Vector Analysis

| Quality Vector | Show-Stopper Requirement | Rationale |
|----------------|-------------------------|-----------|
| Efficiency | None — no show-stopper; obligations covered by `cpt-insightspec-v3-nfr-efficiency` | A few small tables and checks fit the parent footprint |
| Reliability | The upgrade MUST lose nothing, `cpt-insightspec-v3-sharing-nfr-upgrade-integrity` | A dashboard drawing nothing, or gone from its folder, is lost work nobody notices |
| Performance | None — no show-stopper; obligations covered by `cpt-insightspec-v3-sharing-nfr-list-latency` and `cpt-insightspec-v3-nfr-performance` | Visibility through grants and folders must not slow lists |
| Security | A dashboard or folder MUST reach only those `cpt-insightspec-v3-sharing-fr-visibility` names, `cpt-insightspec-v3-sharing-nfr-isolation`; a member MUST reach nothing beyond the dashboards they see, `cpt-insightspec-v3-sharing-nfr-reach`; access changes MUST be traceable, `cpt-insightspec-v3-sharing-nfr-audit` | Privacy is the feature's one promise, sharing a page must not hand over the library behind it, and an audience widened through everyone or a folder must be explainable |
| Versatility | None — no show-stopper; obligations covered by `cpt-insightspec-v3-nfr-versatility` | One rule serves every surface |

### 6.2 NFR Inclusions

#### Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-nfr-isolation`

**Vector**: Security

Nothing about an invisible dashboard or folder (existence, name, owner, tags, folders, entries, grants, content) **MUST** reach a person, except the hidden count of `cpt-insightspec-v3-sharing-fr-library` and the orphaned lists of `cpt-insightspec-v3-sharing-fr-transfer`.

**Threshold**: beyond those, zero disclosure in lists, totals, searches, reads, refusals, pins, folders, entries, tags, counts, caps, dependents, usage reports, chat context and MCP tools, to members and administrators alike. An invisible dashboard or folder answers exactly like a missing one. A filing entry shows its dashboard to nobody who could not see it before. A refused change reveals nothing new.

**Rationale**: The feature is only as good as its privacy.

#### Bounded Reach

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-nfr-reach`

**Vector**: Security

A member **MUST** reach nothing in the Custom zone but the dashboards they see, what those dashboards draw, their pins, their own folders and the folders shared with them.

**Threshold**: every library, definition-creation, direct metric-run, drilldown, chat, MCP, orphaned, transfer, sharing, alert, dataset and ownerless surface refuses a member as not permitted, whatever level they were granted; a widget drawn through a dashboard, the one way a member runs a metric, is one that dashboard holds; no message to a member names a warehouse table or column, or a dataset. A member does read the widgets a visible dashboard holds, which name their metric and detail metric and the result columns they draw.

**Rationale**: A shared page is a window onto chosen data, not a key to the warehouse.

#### Traceable Access Changes

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-nfr-audit`

**Vector**: Security

Every grant, change of level, withdrawal, entry added or removed, and transfer **MUST** be recorded.

**Threshold**: each is logged with actor, dashboard or folder, person or everyone concerned, level where one applies, and time; a grant stores who granted it and an entry who placed it.

**Rationale**: Who can see or change what, and through which grant or folder, must be explainable afterwards.

#### Upgrade Integrity

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-nfr-upgrade-integrity`

**Vector**: Reliability

The move to identities **MUST** finish intact, also when re-run after an interruption, while the previous release keeps serving; that release's writes **MUST NOT** land unconverted or duplicated.

**Threshold**:

- Counts of definitions, pins, folder placements, tags and alerts match before and after, each placement now an entry.
- Every reference that resolved by name resolves by identity: dashboards draw the same widgets in the same order and sit in the same folders, alerts watch the same metric.
- A run stopped at any step and restarted ends in the same state.
- A previous-release write after the keys change is refused, not stored; a folder move it makes after the placements are copied has no effect.

**Rationale**: The upgrade rewrites every stored reference at once.

#### List Latency

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-nfr-list-latency`

**Vector**: Performance

Listing and opening dashboards and folders **MUST** meet the parent's targets with visibility applied, folders included.

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

**Breaking Change Policy**: Routes and tools move from names to identities together with the portal, chat and MCP in one release; routes by name, and the one-folder filing of a dashboard, are removed. See [DESIGN](./DESIGN.md).

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

1. The owner grants the colleague the viewer level; the colleague finds, opens and pins it, files it in a folder of their own, and its widgets draw.
2. The colleague cannot open the metrics or widgets behind it on their own.
3. The owner withdraws the grant; it leaves the colleague's list, pins and folders, and returns there if granted again.

**Postconditions**: Only the owner and its grantees saw it.

**Alternative Flows**: A person identity does not know is not offered. Granted the editor level, an administrator colleague changes the dashboard and shares it onward; a member colleague only views it.

#### Share a Team Folder

- [ ] `p1` - **ID**: `cpt-insightspec-v3-sharing-usecase-team-folder`

**Actor**: `cpt-insightspec-v3-sharing-actor-owner`

**Preconditions**: An administrator owns a folder, and owns or holds editor grants on the dashboards they mean to place in it.

**Main Flow**:

1. The owner places the dashboards in the folder.
2. They grant the folder to the team at the viewer level and to a fellow administrator at the editor level, after being told which people gain access to which dashboards.
3. The team sees the folder and every dashboard in it; the fellow administrator edits those dashboards and places more.
4. A team member files one of those dashboards in a personal folder of their own; nobody else gains anything.

**Postconditions**: One grant reaches every dashboard placed in the folder; removing an entry or withdrawing a grant ends what it gave.

**Alternative Flows**: Someone who can edit the folder but only views a dashboard, or edits it only through another folder, places it there; the folder's audience gains nothing, and only those who see the dashboard anyway find it in the folder.

#### Keep an Orphaned Dashboard in Use

- [ ] `p2` - **ID**: `cpt-insightspec-v3-sharing-usecase-transfer`

**Actor**: `cpt-insightspec-v3-sharing-actor-admin`

**Preconditions**: The owner of a dashboard, or of a folder with grants, no longer holds the administrator role.

**Main Flow**:

1. An administrator finds it in the orphaned list.
2. They transfer it to an administrator, possibly themselves.

**Postconditions**: An administrator owns it; its grants and entries stay.

## 9. Acceptance Criteria

- [ ] Tests prove every requirement in §5 and §6.2 on each surface it names, `cpt-insightspec-v3-sharing-nfr-isolation` as an administrator and as a member who is neither owner nor grantee of the dashboard or of any folder carrying it.
- [ ] `cpt-insightspec-v3-sharing-fr-render` and `cpt-insightspec-v3-sharing-nfr-reach`: a member opens a visible dashboard and its widgets draw; the same member is refused on every surface the reach threshold names, also when granted the editor level; a widget the dashboard does not hold answers as missing; a failing widget tells the member nothing of its warehouse table or columns, or its dataset.
- [ ] `cpt-insightspec-v3-sharing-fr-entries`: a folder's viewer sees a dashboard carried into it and not one only filed there; removing the carrying entry ends that access; a folder's editor edits what it carries.
- [ ] `cpt-insightspec-v3-sharing-fr-share` and `cpt-insightspec-v3-sharing-fr-share-onward`: a grant to everyone reaches a member with no other grant; an editor adds, changes and withdraws grants, everyone's included, but cannot delete the dashboard or withdraw the owner.
- [ ] `cpt-insightspec-v3-sharing-fr-unshare`: a pin and an entry of a person whose grant is withdrawn reappear when access returns.
- [ ] `cpt-insightspec-v3-sharing-fr-entries`: an editor whose editor grant is withdrawn or lowered, or whose everyone grant goes, no longer reaches the dashboard through a folder they placed it in, nor does that folder's audience; an editor whose access comes only through a folder files.
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

- **D1.** Administrators author; any signed-in person may view. Member authoring over the shared library is the next step.
- **D2.** Superseded by D13.
- **D3.** The administrator role shows no private dashboard or folder.
- **D4.** Orphaned means the owner lost the role, which covers a leaver when leaving ends it.
- **D5.** Ownerless rows stay; the operator, not a claim feature, assigns owners.
- **D6.** Tag names are unique per owner, since tags are applied by name and created on first use; a dataset's unique name stays its identity, since outside producers address datasets by name, and unowned datasets disclose nothing. Existing definitions keep their former name as their identity, so nothing stored is rewritten.
- **D7.** Superseded by D16.
- **D8.** Share targets are any person identity knows, or everyone (D15); the sharer is always an administrator.
- **D9.** No sharing, withdrawal or transfer through the chat or MCP.
- **D10.** A viewer sees what the dashboard's metrics return, over any time window the page can ask for, with no per-viewer scoping; a grant is its owner or editor vouching for that person, or for everyone.
- **D11.** Members get no drilldown: a detail metric returns rows the dashboard does not show.
- **D12.** A member's errors are generic; the cause goes to the log.
- **D13.** A grant is at the viewer or the editor level; an editor changes everything about a dashboard but deleting it.
- **D14.** Editors share onward: managing grants is not reserved to the owner.
- **D15.** A dashboard or folder may be granted to everyone, as an explicit choice that is never the default. This reverses the earlier exclusion of public dashboards; links stay excluded.
- **D16.** Folders are shareable containers: a dashboard sits in any number of folders, and an entry carries the folder's audience only as D20 allows.
- **D17.** Every signed-in person, members included, may own personal folders and file in them what they see; only administrators share folders.
- **D18.** Withdrawing a grant keeps the person's pins and entries, hidden until access returns.
- **D19.** The editor level applies only to administrators; members are capped at viewing.
- **D20.** An entry carries only while its placer owns the dashboard or holds an editor grant on it, directly or through everyone, so withdrawing a grant or transferring a dashboard ends what that person's entries gave; editing through a folder never carries onward.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Searching a body by the name of a metric or widget it uses | No match | Search the definition's own name; dependents show its users |
| Identity unreachable | Nobody identified | Server error, never allowed, like the administrator check now |
| An owner loses the role briefly | Dashboard or folder transferred meanwhile | Accepted; the new owner can grant it back |
| A viewer sees data the rest of the portal would scope away from them | Disclosure beyond the viewer's own team | D10: owners and editors choose who sees a dashboard; per-viewer scoping is its own feature |
| A member probes the library through dashboard-scoped runs | Reach beyond the shared page | `cpt-insightspec-v3-sharing-nfr-reach`: a run names a widget the dashboard holds or answers as missing |
| A grant to everyone | Every signed-in person, members included, sees the dashboard or every dashboard a folder carries | D15: an explicit, never-preselected choice; recorded per `cpt-insightspec-v3-sharing-nfr-audit`; any owner or editor withdraws it |
| A carrying entry in a folder that is shared later | The dashboard reaches the folder's new audience | `cpt-insightspec-v3-sharing-fr-entries`: the person sharing the folder is told who gains access first |
| An editor shares onward wider than the owner meant | Audience beyond the owner's choice | D14: the owner chooses editors, sees who granted each grant, and withdraws any |
