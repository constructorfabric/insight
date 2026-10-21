---
status: accepted
date: 2026-10-21
---

# ADR-0010: Definitions Are Addressed by Generated Identity, Names Are Labels

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Keep names unique across the installation](#keep-names-unique-across-the-installation)
  - [Unique per owner, keyed by owner and name](#unique-per-owner-keyed-by-owner-and-name)
  - [Generated UUIDv7, names as labels](#generated-uuidv7-names-as-labels)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-v3-adr-definitions-addressed-by-identity`

## Context and Problem Statement

Metrics, widgets, dashboards and datasets are keyed by an installation-wide unique name, which every reference, pin, tag link and alert uses. A private dashboard would reserve its name against everyone, and the refusal would reveal it.

## Decision Drivers

* A private dashboard discloses nothing, not even its existence, to anyone who may not see it, beyond the two disclosures the [sharing PRD](../sharing/PRD.md)'s isolation requirement allows: a count of hidden dashboards that use a widget or metric, and the orphaned list.
* Two people can use the same name.
* A rename rewrites no reference.
* One identifier per entity everywhere, per the [UUIDv7 primary key ADR](../../../../shared/glossary/ADR/0001-uuidv7-primary-key.md).

## Considered Options

* Keep names unique across the installation.
* Make names unique per owner, keyed by owner and name.
* Key every definition, folder and tag by a generated UUIDv7.

## Decision Outcome

Chosen option: "Key by a generated UUIDv7", because only then does a private dashboard reserve nothing, and renames rewrite nothing.

Two names stay unique:

* A tag's, per owner: tags are applied by name and created on first use, so a name must resolve to one tag.
* A dataset's, installation-wide: outside producers send records to a dataset by name, and its physical table is named after it. Datasets have no owner, so their names disclose nothing private.

### Consequences

* Good, because no collision reveals a private dashboard, and every reference points at a key that never changes.
* Bad, because every route, tool, chat proposal and portal path changes at once, and links by name break.
* Bad, because the upgrade spans statements MariaDB cannot put in one transaction, so it must resume after an interruption.
* Bad, because the assistant must read an id to replace a definition; creating a same-named one is no replacement.

### Confirmation

* No definition, folder or tag table has a unique key on `name`, except `tags` on owner scope and name, and `datasets` on name.
* Upgrade tests on a seeded previous-release schema assert that every reference resolving by name, in bodies, pins, tag links and alerts, becomes an id, and a dangling one is kept and reads as missing.
* No route or MCP tool takes a name where it means a definition.

## Pros and Cons of the Options

### Keep names unique across the installation

* Good, because nothing changes.
* Bad, because it fails the first two drivers.

### Unique per owner, keyed by owner and name

* Good, because names stay handles the assistant can type.
* Bad, because a shared dashboard and one of mine can share a name, a transfer can collide with the new owner's names, and unowned widgets, metrics and datasets need another key rule.

### Generated UUIDv7, names as labels

* Good, because the key never collides.
* Bad, because it is the largest change.

## Traceability

- **PRD**: [Dashboard Ownership and Sharing](../sharing/PRD.md), `cpt-insightspec-v3-sharing-fr-identity`
- **DESIGN**: [Dashboard Ownership and Sharing](../sharing/DESIGN.md), [Upgrade Procedure](../sharing/DESIGN.md#upgrade-procedure)
