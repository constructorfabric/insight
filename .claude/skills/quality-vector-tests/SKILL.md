---
name: quality-vector-tests
description: >-
  Bind Insight tests to the requirements they prove, the canonical Studio way: a
  FEATURE's Definitions of Done are what tests verify, a test names the DoD or flow
  ID it proves, and quality vectors stay on the PRD's NFRs. Use when choosing the
  suite for a check, linking a test to its FEATURE, checking a DoD item off, or
  reviewing whether a feature's requirements are proven.
---

# Tests for Insight features

The trace follows the canonical Studio SDLC kit and adds nothing to the FEATURE:

`PRD FR/NFR → DESIGN allocation → FEATURE section 1.2 + DoD items → test naming the DoD or flow ID`

- A FEATURE ends at section 5 Definitions of Done and section 6 Acceptance
  Criteria. It carries no testing section, no scenario list and no vectors.
- Quality vectors live on the PRD: the section 6.1 Quality Vector Analysis and
  the NFRs in 6.2, with DESIGN's NFR allocation saying how each is verified. A
  functional requirement carries no vector. The
  [quality-vector authoring guide](../../../.cf-studio/config/kits/sdlc/guides/quality-vectors.md)
  owns those definitions.
- The pytest vector marker every stand API/UI test carries selects tests with
  `-m`; it binds nothing to a requirement.

## 1. Resolve the feature and what it must prove

Read the registered FEATURE, its DECOMPOSITION entry, PRD and DESIGN. Section 1.2
lists the FR/NFR IDs the feature implements, including inherited NFRs; confirm
each inherited ID resolves upstream and its target still applies.

The DoD items in section 5 are the unit a test proves. Section 6 Acceptance
Criteria is the feature's readable completion checklist: improve an unclear
criterion when that is in scope, but do not number it or turn it into a coverage
map. An NFR is proven by the verification its DESIGN allocation names.

An issue is a planning surface. Plan coverage there with `scope-feature-tests`;
the long-lived home is the FEATURE. If no FEATURE exists, say the PRD → FEATURE →
test chain is incomplete rather than inventing a feature or DoD ID.

## 2. Ground the expected outcomes

Agreed requirements define expected outcomes. Code inspection locates the real
boundaries, suitable suites, existing tests and specification disagreements; it
must not silently redefine the oracle to match current behaviour. Read
`scope-feature-tests` for deeper scope analysis.

For existing code, distinguish a retrospective test plan from new capability.
For a port or migration, lead with parity on the same synthetic inputs. Tag
differences `exact`, `known-diff(direction)`, or `merge` where appropriate.

Keep unresolved product decisions in feature context, with owners and resolution
points. A check with no agreed upstream requirement is one of those decisions.
Missing infrastructure is a gap, never a reason to call an obligation
inapplicable. Do not manufacture targets, fixtures, supported-source counts, or
degraded behaviour.

## 3. Choose the suite

Choose the cheapest existing suite capable of falsifying the check:

| Target component | Suite tags, cheapest adequate first |
|---|---|
| Frontend | `fe-unit` (vitest `unit` project) → `fe-component` (vitest `storybook` project) → `stand-ui` (`tests/stand/ui`) |
| Serving / analytics | `rust-unit` (inline `#[cfg(test)]`) → `metric-spec` (`tests/datapath/metrics/<class>`) → `stand-api` (`tests/stand/api`) |
| Authentication | `rust-unit` → `auth-rig` (`src/backend/services/authenticator/tests`) → `stand-api` / `stand-ui` |
| Identity | `rust-unit` → `identity-e2e` (`tests/datapath/identity`) → `stand-api` |
| Ingestion | `connector-tests` (`src/ingestion/connectors/*/*/tests`) → `dbt-tests` (`src/ingestion/dbt/tests`) → `metric-spec` |
| Cross-cutting | `ci-static` (scans and gates in `.github/`) or `manual` |
| Already in operation | `observed` — read from operational telemetry rather than run |

Verify tools before naming them: inspect the relevant test directory, package
scripts or CI workflows. `scripts/counts.sh` takes repo-wide denominators
and reports MOVED rather than a misleading zero when a source has shifted; take a
count from it rather than quoting one. A browser test must need browser-observable
behaviour.

A stand API/UI test also carries exactly one vector marker under
`tests/pyproject.toml`: the vector of what that test asserts, per
[vector-mapping.md](references/vector-mapping.md).

`observed` covers an NFR that operational telemetry already measures on real
traffic, so no test is run: per-endpoint latency percentiles, request and error
rates, and per-service reserved-versus-used CPU and memory are all recorded
continuously. Prefer it to `manual` for Performance and Efficiency — a synthetic
run at an invented scale is weaker evidence than a percentile from real use.

An `observed` line states the panel or query, the window, and the environment it
was read from. Without those it is an assertion, not evidence. It is lagging, so
it validates after the fact and cannot gate a merge; and it describes the
environment measured, so a lightly loaded stand says little about a
production-scale organisation. Where load-test traffic shares the same logs or
metric store, say whether it was excluded.

Keep `manual` for a check that needs a procedure someone performs. If a check
needs a lane that does not exist, record the decision in feature context with its
owner.

## 4. Link a test to the FEATURE

1. Verify that the assertions prove the whole DoD item or flow, not a neighbour
   of it.
2. Name the ID in the test — a docstring or comment line such as
   `Verifies cpt-ir-dod-manual-resolution-idempotency`; for a YAML metric spec,
   its description field. Do not invent marker kinds: the `@cpt-{flow|algo|state|dod}`
   scope and begin/end markers belong in implementation code under FULL
   traceability, not in tests.
3. Check a DoD item `[x]` only when its implementation is in place, the tests
   naming it pass, and — under FULL traceability — its code markers exist.
4. Record passing evidence separately: tested revision, fixture/version, run
   command, conditions and result. Skipped tests, missing personas and absent
   source fixtures are not passing evidence.

For an existing test, link only the behaviour its assertions actually prove.

## 5. Validate

```sh
cfs toc docs/<path>/FEATURE.md
cfs validate --artifact docs/<path>/FEATURE.md
cfs validate-toc docs/<path>/FEATURE.md
```

Review the chain in both directions: PRD IDs resolve; DESIGN allocates each NFR
and names its verification; the FEATURE declares its requirements; each DoD item
has a test naming it, or an open decision with an owner. Explain missing or
pending evidence without certifying an unmet requirement. Do not use a ratio of
requirements to tests as a quality or traceability gate.
