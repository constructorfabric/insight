---
name: quality-vector-tests
description: >-
  Bind Insight tests to the requirements they prove and derive a FEATURE's
  Testing section from those bindings. Use when a feature needs its section 7,
  when a test should cite the FR/NFR it verifies, when reviewing which
  requirements have no test, or when filling the Collected today and Note cells
  of the quality-metrics table. Keeps Acceptance Criteria in the kit's form.
---

# Bind tests to requirements

A test proves a requirement by citing it. Nothing is authored twice: the PRD
states the requirement, the test cites it, and
`scripts/ci/feature_testing.py` renders FEATURE section 7 from the citations.

`PRD FR/NFR → FEATURE 1.2 Requirements → test citing the ID → generated 7.1 / 7.2`

Read the [quality-vector guide](../../../.cf-studio/config/kits/sdlc/guides/quality-vectors.md)
for the vector definitions and the generative rule (a vector produces an NFR;
it is never a requirement). A functional requirement never carries a vector.

## 1. Find the requirement a test proves

Read the FEATURE's section 1.2 `**Requirements**` list and the PRD entries it
names. Pick the exact ID whose MUST sentence the test's assertions falsify. A
DoD ID (`cpt-*-dod-*`) is allowed when the claim is implementation-level. If
no agreed requirement states the claim, that is an unresolved product
decision: record it in feature context with an owner, do not invent an ID.

## 2. Cite it from the test

Put one scope marker per cited requirement on the line above the test, in the
file's comment syntax:

```python
# @cpt-test:cpt-ir-fr-operator-bind:p1
@pytest.mark.reliability
def test_bind_unseen_account(...):
```

```ts
// @cpt-test:cpt-insightspec-v3-fr-create-alerts:p1
it("creates an alert without a code change", ...)
```

A test may cite several IDs; a measurement run that feeds two metrics cites
both NFRs. Keep the native pytest vector marker on stand API/UI tests: it
selects tests with `-m` and is unrelated to the citation. The phase suffix
is the kit's grammar; use `p1`.

Choose the cheapest suite that can falsify the claim, and prefer an end-to-end
lane for a functional requirement:

| Target component | Suite, cheapest adequate first | End to end |
|---|---|---|
| Frontend | `fe-unit` → `fe-component` → `stand-ui` (`tests/stand/ui`) | `stand-ui` |
| Serving / analytics | `metric-spec` (`tests/datapath/metrics`) → `stand-api` (`tests/stand/api`) | `metric-spec`, `stand-api` |
| Authentication | `auth-rig` (`src/backend/services/authenticator/tests`) → `stand-api` / `stand-ui` | `stand-api`, `stand-ui` |
| Identity | `identity-e2e` (`tests/datapath/identity`) → `stand-api` | `identity-e2e`, `stand-api` |
| Ingestion | `connector-tests` → `dbt-tests` → `ingestion-e2e` (`src/ingestion/tests`) → `metric-spec` | `ingestion-e2e`, `metric-spec` |

The suite is derived from the test's path by the tool; nothing is tagged by
hand. Only trees registered as `[[systems.codebase]]` in
`.cf-studio/config/artifacts.toml` are scanned; a test elsewhere is invisible,
including an inline Rust `#[cfg(test)]` module.

## 3. Generate section 7

```sh
python3 scripts/ci/feature_testing.py docs/<path>/FEATURE.md --write
python3 scripts/ci/feature_testing.py docs/<path>/FEATURE.md --gate
cfs toc docs/<path>/FEATURE.md
```

Table 7.1 lists each functional requirement with its citing tests, suites and
whether any citation is end to end. Table 7.2 lists each NFR with its vector
(from the PRD's 6.1 row or `**Vector**` line), metric (the NFR heading),
target (its `**Threshold**`), `Collected today`, source and `Note`.

Author only two cells:

- `Collected today`: the measured value with its date, environment and window,
  such as `p95 1.38 s, cf-prod, 2026-08-19, 1 user`. Leave empty when nothing
  was collected; never invent a reading.
- `Note`: for a requirement with no citing test or no source, the reason and
  the owner, such as `split e2e pending, owner KT`. The gate reads an empty
  Note on an uncited row as a gap.

Sources for an NFR are the citing tests plus the DESIGN NFR-allocation
`Verification Approach` cell. For an observed metric that cell must name the
dashboard, panel, query, window and environment; `Insight · API endpoints,
p95 by route, 7d, insight-dev` is a source, `latency measurements` is not.
Observed evidence is lagging and cannot gate a merge; say which environment it
describes.

`scripts/counts.sh` still takes repo-wide denominators for a target written
as a fraction, and reports MOVED rather than a misleading zero.

## 4. Review a feature's coverage

Run `--gate` and read the gaps: an uncited functional requirement, an NFR with
no source, or an `unknown id` (a citation of an ID that was renamed, usually to
`-v2`). A unit-only citation is not a gap; it is a weaker proof worth naming
in review. Do not add a scenario, a ratio or a vector to close a gap: add a
test, a source, or a Note with an owner.

When an issue is the planning surface and no FEATURE exists yet, list the
requirement IDs the work will cite and say the chain is incomplete; the tables
are generated once the FEATURE exists.

## 5. Validate

```sh
python3 scripts/ci/feature_testing.py --check
cfs validate --artifact docs/<path>/FEATURE.md
cfs validate-toc docs/<path>/FEATURE.md
```

`--check` fails only for FEATUREs that already carry the generated block and
are stale. `cfs` validates artifact references; it does not index `@cpt-test`
citations while the system's artifacts are DOCS-ONLY, which is why the tool
scans the registry itself.
