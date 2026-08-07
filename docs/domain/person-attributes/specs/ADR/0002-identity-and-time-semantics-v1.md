---
status: proposed
date: 2026-08-05
decision-makers: Insight engineering
---

# ADR-0002: Separate Corrective Account Identity from Temporal Attribute Facts


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Email Resolution for All Facts](#email-resolution-for-all-facts)
  - [Effective-Dated Person Assignment](#effective-dated-person-assignment)
  - [Corrective Account Assignment with Temporal Attributes](#corrective-account-assignment-with-temporal-attributes)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-person-attributes-adr-identity-and-time-semantics`
## Context and Problem Statement

Connector attribute claims identify native source accounts, while metric results and groups identify canonical people. The current identity model records the latest source-account-to-person decision, and a developing identity workflow is expected to preserve that corrective behavior. Attribute values, however, must retain the job title, department, office, or manager relationship that was valid during each metric period.

The architecture must decide how current account assignment, historical attribute facts, email-only metric observations, and period-crossing people-like comparisons interact without making #2028 depend on the unfinished identity workflow.

## Decision Drivers

- Stable source-account identifiers are stronger than email for connector attribute claims.
- Human identity corrections should repair all retained facts for an account.
- A person's historical attributes must not be rewritten to their current values.
- The current `identity.identity_persons` snapshot already exposes latest account bindings.
- Some metric observations currently contain only email and still require resolution.
- #2028 must be able to adopt the future identity workflow without changing analytical claims or APIs.
- Period-crossing comparisons must not hide changes in the subject's peer definition.

## Considered Options

- Resolve every attribute and metric observation through the current email snapshot.
- Introduce effective-dated person assignment and evaluate both assignment and attributes as-of every observation.
- Keep attribute facts source-account-scoped and effective-dated, resolve them through the current corrective assignment during each cohort query, and retain email resolution only for observations without stable account identity.

## Decision Outcome

Chosen option: **Current corrective source-account assignment plus temporal account facts**, because it matches the current and planned identity semantics while preserving the history #2028 actually needs. Attribute facts remain keyed by `(tenant, source type, source instance, source account ID)` and contain no canonical `person_id`. Every cohort query joins them to the current assignment projection. Reassigning an account therefore changes attribution after that small projection refresh without rebuilding attribute history. The fact's value intervals are not changed.

Email resolution remains an adapter for metric observations that do not carry stable source-account identity. Both adapters resolve to the same canonical person ID, but email-resolved metric facts retain their own identity watermark and publication cadence. The current account-assignment revision and metric identity watermark are recorded in request diagnostics rather than pretending the two paths update atomically.

When a people-like subject changes a selected attribute during the common reliable portion of the requested period, analytics returns maximal stable temporal segments and reports the covered period. A shorter covered period is the required `available_partial` result variant rather than an ordinary available result. The comparison subject remains in the matching aggregate, consistent with the epic's worked example, and person-grain membership prevents double counting. Named-group conditions remain fixed and evaluate changing membership over the covered period.

### Consequences

- Good, because attribute ingestion does not regress to mutable or shared email identity.
- Good, because a human correction repairs historical attribution without rewriting source facts.
- Good, because account reassignment does not wait for the connector/dbt attribute build.
- Good, because job, office, and hierarchy history remain period-correct.
- Good, because the future identity workflow can replace the assignment producer behind a stable projection contract.
- Good, because existing email-only metric models can migrate independently.
- Bad, because assignment history is not effective-dated; the latest correction applies retroactively.
- Bad, because a long people-like request can return several comparison segments.
- Bad, because group membership and metric coverage may differ when metric aliases remain unresolved.
- Risk: a native account ID reused between humans would reattribute earlier claims incorrectly. Supported connectors must treat account IDs as non-reusable; shared and service accounts are excluded. If that invariant fails, a new ADR must introduce effective-dated assignment.
- Risk: email-resolved metric facts can lag the current account assignment. Request diagnostics record both revisions, and `measured_n` reflects only currently available canonical metric facts.

### Confirmation

The decision is confirmed by design and implementation review showing:

- Attribute claims join through stable source-account keys and never fall back to email when that key exists.
- The initial assignment projection derives the latest `value_type = 'id'` record from `identity.identity_persons`.
- Account attribute values contain no canonical `person_id`.
- Refreshing the current assignment projection changes subsequent cohort attribution without rebuilding account values.
- Email resolution remains isolated to observations that lack stable account identity.
- People-like results split when selected subject values change inside the period.
- Results use distinct full and partial availability variants; partial results identify requested versus covered period.
- Peer counts and aggregates include the comparison subject once.
- Named groups retain fixed conditions while qualifying observations by temporal membership.

## Pros and Cons of the Options

### Email Resolution for All Facts

Every attribute claim and metric observation joins the latest normalized email-to-person snapshot.

- Good, because one existing resolver serves every input.
- Good, because implementation is initially small.
- Bad, because email can change, collide, or be absent while a stable account ID exists.
- Bad, because source provenance is discarded during resolution.
- Bad, because using email for stable account claims would make future identity migration harder.

### Effective-Dated Person Assignment

Account-to-person assignment and attribute values both carry historical validity and are evaluated as-of each fact.

- Good, because it can represent account reuse and assignment history exactly.
- Good, because historical attribution never changes after a correction.
- Bad, because it conflicts with the current human-decision model where corrections intentionally repair all history.
- Bad, because no current assignment event history exists to backfill trustworthy intervals.
- Bad, because it expands #2028 into a new identity semantics project and blocks delivery on the unfinished workflow.

### Corrective Account Assignment with Temporal Attributes

The latest source-account decision is joined to all retained account facts at query time; each fact keeps its own effective-dated business value. Email remains only for observations without a stable account key.

- Good, because it matches current identity evidence and planned correction behavior.
- Good, because it preserves the business history needed for peer grouping.
- Good, because it creates a stable adapter boundary for the future identity workflow.
- Good, because account and email resolution converge on canonical person IDs before metric aggregation.
- Good, because identity correction requires no account-attribute rewrite.
- Bad, because native account reuse cannot be represented safely.
- Bad, because two resolution adapters coexist during migration.

## More Information

- **Scope:** Account resolution, email-only metric compatibility, and temporal comparison semantics. Identity administration is outside this decision.
- **Read path:** Current assignment avoids an identity-history join; attribute and metric time joins remain in one ClickHouse statement.
- **Operations:** Assignment revision, lag, unresolved counts, and value/metric watermarks are observable. Values rebuild independently of assignment.
- **Compatibility:** Email-only metrics keep their adapter; a future identity workflow replaces only the typed assignment producer.
- **Review trigger:** A source reuses account IDs, Identity adopts effective-dated assignment, or all metric producers gain stable account identity.

## Traceability

- **Requirements**: [GitHub issue #2028](https://github.com/constructorfabric/insight/issues/2028) — requires period-correct person grouping and comparison.
- **DESIGN**: [Person Attributes and Cohorting](../DESIGN.md) — defines the assignment projection, temporal values, and membership behavior.
- **Related ADR**: [ADR-0001](./0001-attribute-data-ownership-v1.md) — defines where current assignment and account-scoped temporal facts are published.

This decision directly constrains:

- `cpt-person-attributes-principle-source-account-resolution`
- `cpt-person-attributes-principle-temporal-segmentation`
- `cpt-person-attributes-constraint-corrective-assignment`
- `cpt-person-attributes-component-assignment-publisher`
- `cpt-person-attributes-component-account-value-builder`
- `cpt-person-attributes-seq-people-like`
