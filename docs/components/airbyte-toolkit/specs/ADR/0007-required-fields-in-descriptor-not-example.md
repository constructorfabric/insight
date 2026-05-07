---
status: accepted
date: 2026-05-05
decision-makers: platform-engineering
---

# ADR-0007: Required Secret Fields Declared in descriptor.yaml, not example.yaml


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Keep annotations on `*.yaml.example`](#option-a--keep-annotations-on-yamlexample)
  - [Option B — Move to `descriptor.yaml.secret.required_fields`](#option-b--move-to-descriptoryamlsecretrequiredfields)
  - [Option C — Introduce a separate `validation.yaml` per connector](#option-c--introduce-a-separate-validationyaml-per-connector)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-required-fields-in-descriptor-not-example`

## Context and Problem Statement

PR #281 (Phase 1) introduced 16 `secrets/connectors/*.yaml.example` files annotated with `insight.cyberfabric.com/required-fields` and `insight.cyberfabric.com/optional-fields`, which the early validator parsed to build its required-fields list. Operators report two issues:

1. The annotations break the property "an example is a valid Secret manifest after substituting placeholder values" — `kubectl apply -f *.yaml.example` is a useful operator UX (sed → apply), but the annotations carry semantics the cluster shouldn't see and litter the runtime metadata.
2. The descriptor `connectors/<name>/descriptor.yaml` is already the canonical contract for the connector (version, schedule, source-config schema). Splitting "required fields" between descriptor and example creates two sources of truth and operator confusion.

Where does the "required fields" contract live, and how does the validator and the reconcile loop consume it?

## Decision Drivers

- **Single source of truth** for the connector contract.
- **Operator UX**: `secrets/connectors/*.yaml.example` must be a plain Kubernetes Secret manifest, reusable verbatim (substitute placeholders → `kubectl apply`).
- **Validator simplicity**: one parser path, not two.
- **Forward extensibility** of the connector contract (e.g., future `secret.optional_fields`, `secret.regex`).
- **Migration cost**: any change requires a one-pass edit of all 16 connectors; favour the option with the smallest blast radius downstream.

## Considered Options

- **Option A** — Keep annotations on `*.yaml.example` (status quo from PR #281).
- **Option B** — Move required-fields to `descriptor.yaml.secret.required_fields` (CHOSEN).
- **Option C** — Introduce a separate `connectors/<name>/validation.yaml` per connector.

## Decision Outcome

Chosen option: **Option B — required-fields declared in `descriptor.yaml.secret.required_fields`**.

**Justification**: descriptor.yaml gains a top-level `secret:` block:

```yaml
secret:
  required_fields:
    - <field-1>
    - <field-2>
```

The validator reads required fields from descriptor only. `secrets/connectors/*.yaml.example` is stripped of `insight.cyberfabric.com/required-fields` and `insight.cyberfabric.com/optional-fields` annotations. User-facing annotations (`connector`, `tenant`, `schedule`) remain.

Validation rules (frozen by this ADR):

- Required field missing in K8s Secret `stringData`/`data` → INVALID → WARN+skip (do NOT cascade-delete; connection survives until human intervention).
- K8s Secret entirely missing → cascade-delete (connection + source + definition if `ref_count == 0` + per-connector CronWorkflow `${connector}-${tenant}-sync`).
- Bronze ClickHouse data preserved in both cases.

### Consequences

- **Good**, because there is one source of truth for the connector contract.
- **Good**, because examples become reusable verbatim by operators.
- **Good**, because the validator simplifies (one parser path).
- **Good**, because future fields on the contract (e.g., `secret.optional_fields`, `secret.regex`) extend the same `secret:` block.
- **Bad**, because Phase 16 must touch all 16 descriptor.yaml files in one pass.
- **Bad**, because examples lose self-documenting "this field is required" comments — the new convention is "descriptor is the manifest of the contract; example is a recipe to satisfy it".

### Confirmation

- DoD `cpt-insightspec-dod-reconcile-required-fields-validated-from-descriptor` (FEATURE-reconcile, Phase 7): apply Secret missing one of the declared required fields → reconcile emits `WARN skip ${connector}: missing field <name>`; connection NOT touched. Add the field → reconcile updates the connection without warn.
- Static check in CI: every `secrets/connectors/*.yaml.example` MUST `kubectl apply --dry-run=server -f -` cleanly (no annotation rejection by admission webhooks).
- Static check in CI: every `connectors/*/descriptor.yaml` MUST contain a non-empty `secret.required_fields` list.

#### Correction (2026-05-07): Secret name is NOT the connector slug

An earlier draft of this ADR stated "Secret name == connector slug". This was inconsistent with the deployed convention. The authoritative naming pattern in the cluster is `insight-${connector}-${source-id}` (e.g. `insight-m365-main` for connector `m365`, source-id `main`). The canonical lookup is via the K8s Secret **annotation** `insight.cyberfabric.com/connector` whose value equals the connector slug — never by Secret name match. The matcher is `disc_match_descriptor_to_secret(connector_name)` in `lib/discover.sh`; every consumer (`valsec_*`, `reconcile_*`, `adopt_*`) MUST use it rather than `kubectl get secret ${connector_slug}` directly.

This correction was prompted by a manual cluster smoke (Phase 18 Step J was skipped) discovering that `valsec_secret_missing_p` falsely reported all 16 connectors as missing because it searched by `kubectl get secret <connector_slug>`, triggering a spurious cascade-delete cascade.

## Pros and Cons of the Options

### Option A — Keep annotations on `*.yaml.example`

Status quo from PR #281: required fields encoded as Secret annotations (`insight.cyberfabric.com/required-fields`).

- Good, because zero change.
- Bad, because example is no longer a plain Secret manifest.
- Bad, because the two-source-of-truth problem persists between annotation strings and any other contract surface.
- Bad, because the validator must parse annotation strings, which are stringly-typed and brittle to comma/whitespace edits.

### Option B — Move to `descriptor.yaml.secret.required_fields`

Descriptor file gains a `secret:` block with a `required_fields` list. Validator reads from descriptor; example is reduced to a plain Secret manifest.

- Good, because descriptor is canonical; example becomes a usable `kubectl apply` recipe.
- Good, because validator code becomes simpler (one parser instead of two).
- Good, because aligns with `cpt-insightspec-fr-secret-validation`.
- Neutral, because schema extends with a new top-level `secret:` block.
- Bad, because Phase 16 must touch 16 descriptors in one pass (manageable).

### Option C — Introduce a separate `validation.yaml` per connector

A third file `connectors/<name>/validation.yaml` with `required_fields`, `optional_fields`, plus future validation rules.

- Good, because clean split between "connector contract" (descriptor) and "validation rules" (validation).
- Bad, because adds a third config file per connector with no concrete future requirement.
- Bad, because descriptor.yaml is already the natural home for the contract.
- Bad, because it does not solve any operator-facing problem; it would introduce one (now operators look in two files).

## More Information

- A future ADR may add `secret.optional_fields` or `secret.regex` to the descriptor `secret:` block — out of scope here.
- The example's user-facing annotations (`insight.cyberfabric.com/connector`, `…/tenant`, `…/schedule`) are preserved; only the deprecated `…/required-fields` and `…/optional-fields` are removed.
- Phase 16 (Phase plan §16) performs the bulk edit across all 16 descriptors and examples.
- Related decisions:
  - `cpt-insightspec-adr-cron-self-run-with-file-persistent-logs` (ADR-0006) — the cron pod that performs the validate-and-skip flow.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses:

- `cpt-insightspec-fr-secret-validation` — the FR.
- `cpt-insightspec-fr-cascade-delete-cronworkflow` — the cascade rule on Secret-missing referenced in the validation policy above.
- `cpt-insightspec-algo-reconcile-validate-secret-required-fields-from-descriptor` — the algorithm in FEATURE-reconcile.
- `cpt-insightspec-dod-reconcile-required-fields-validated-from-descriptor` — the DoD that exercises the policy.
