# Connector mock-server tests

L1 of the connector test ladder (see
[`docs/domain/connector/specs/feature-connector-mock-tests/FEATURE.md`](../../../../docs/domain/connector/specs/feature-connector-mock-tests/FEATURE.md)):
credential-free pytest suites for **nocode** (declarative-YAML) connectors.

## How a nocode connector is tested

A nocode connector has no Python of its own — `connector.yaml` **is** the
implementation, executed by the Airbyte CDK. So the unit under test is the
manifest, and the test boundary is HTTP:

1. `get_source()` instantiates the package's `connector.yaml` **in-process**
   via the CDK's `YamlDeclarativeSource` — the same code path that runs it in
   Airbyte (`source-declarative-manifest` image) — after validating the test
   config against the manifest `spec`.
2. `HttpMocker` intercepts HTTP at the transport layer and serves the suite's
   `fixtures/*.json`. An unmatched request fails the test (no network
   fallthrough); on a passing test every registered matcher must have been hit.
3. `read_stream()` runs a full protocol `read` as a black box and returns the
   typed output — records, state messages, logs.
4. Assertions cover what the manifest declares: pagination stop conditions,
   incremental cursors (state emission + resume-request filtering, via exact
   query matchers), error-handler policy (429 retry / ignored codes),
   `AddFields`/`record_filter` transformations incl. the mandatory
   `tenant_id`/`source_id`/`unique_key` stamping, and record shape against the
   stream schema.

## Layout

```text
src/ingestion/tests/connectors/        # this package (the measured harness)
  connector_tests/                     #   get_source / read_stream / builders /
  meta/                                #   schema asserts; harness's own tests
  harness_plugin.py                    #   collection: meta/ + every nocode suite
src/ingestion/connectors/<cat>/<name>/tests/   # per-connector suites
  conftest.py                          #   sys.path + `from connector_tests.plugin import *`
  config.py                            #   <Name>ConfigBuilder(ConfigBuilder)
  test_<stream>.py                     #   one module per stream
  fixtures/*.json                      #   response bodies (mandatory)
```

CDK (Python) connectors are **not** collected here — they have their own
pyproject, airbyte-cdk pin, and coverage component. A connector suite is
collected when the package has a `connector.yaml` and no `pyproject.toml`.

## Run

```bash
cd src/ingestion/tests/connectors
python3.12 -m venv .venv && .venv/bin/pip install -e '.[dev]'
.venv/bin/pytest                       # harness meta + all nocode suites
.venv/bin/pytest --meta-only           # only the harness's own tests
.venv/bin/pytest --suites-only         # only the connector suites
.venv/bin/pytest ../../connectors/task-tracking/jira/tests   # one suite
.venv/bin/pytest --cov=connector_tests --cov-report term-missing
```

Reference suite: [`task-tracking/jira/tests`](../../connectors/task-tracking/jira/tests)
— plain paginated stream (`jira_projects`) + incremental substream
(`jira_issue_keys`), covering the spec's stream coverage matrix.

## Coverage

Two CI jobs (`scripts/ci/components.py`), kept separate so results are clean:
`connector-tests-harness` runs `--meta-only` (the harness's own unit tests) and
`connector-mock-tests` runs `--suites-only` (the per-connector suites). They
co-trigger via `triggered_by` and their Cobertura reports merge under
`connector-tests-harness` at the shared gate (`scripts/ci/coverage.py`, ≥ 80%
overall and on new code). Manifests are YAML — line coverage measures the
harness; **behavioral** coverage of a connector is the spec's stream coverage
matrix, enforced per suite (a skipped matrix row must carry an explicit skip
reason). Each run ends with a covered-vs-missing table per connector (also
written to the GitHub job summary on CI).

## Conventions

- Freeze the clock (`freezegun`) in any test touching cursors or
  datetime-templated params.
- Fixtures are **mandatory** — every response body lives in `fixtures/*.json`
  (same approach for big and small responses); tests load them via
  `load_fixture(__file__, "name.json", **overrides)` and override only the
  fields the case exercises. Shapes come from real API payloads, values are
  synthetic — never commit real customer data, tokens, or hostnames.
- The `airbyte-cdk` pin must match the `version:` header of the nocode
  manifests (currently the 6.60.x line); bump them in lockstep.
- CDK interpolation literal-evals rendered Jinja values: a numeric-string id in
  `{{ record['id'] }}` becomes an `int` in the emitted record (and in the
  generated schema — cf. `jira_projects.project_id: number`).
