# Connector mock-server tests

L1 of the connector test ladder (see
[`docs/domain/connector/specs/feature-connector-mock-tests/FEATURE.md`](../../../../docs/domain/connector/specs/feature-connector-mock-tests/FEATURE.md)):
credential-free pytest suites for **nocode** (declarative-YAML) connectors. A
suite loads the package's `connector.yaml` in-process through the pinned
`airbyte-cdk`, intercepts HTTP at the transport layer (`HttpMocker` — an
unmatched request fails the test, no network fallthrough), and runs a full
protocol `read` as a black box.

## Layout

```
src/ingestion/tests/connectors/        # this package (the measured harness)
  connector_tests/                     #   get_source / read_stream / builders /
  meta/                                #   schema asserts; harness's own tests
  harness_plugin.py                    #   collection: meta/ + every nocode suite
src/ingestion/connectors/<cat>/<name>/tests/   # per-connector suites
  conftest.py                          #   sys.path + `from connector_tests.plugin import *`
  config.py                            #   <Name>ConfigBuilder(ConfigBuilder)
  test_<stream>.py                     #   one module per stream
```

CDK (Python) connectors are **not** collected here — they have their own
pyproject, airbyte-cdk pin, and coverage component. A connector suite is
collected when the package has a `connector.yaml` and no `pyproject.toml`.

## Run

```bash
cd src/ingestion/tests/connectors
python3.12 -m venv .venv && .venv/bin/pip install -e '.[dev]'
.venv/bin/pytest                       # harness meta + all nocode suites
.venv/bin/pytest ../../connectors/task-tracking/jira/tests   # one suite
.venv/bin/pytest --cov=connector_tests --cov-report term-missing
```

Reference suite: [`task-tracking/jira/tests`](../../connectors/task-tracking/jira/tests)
— plain paginated stream (`jira_projects`) + incremental substream
(`jira_issue_keys`), covering the spec's stream coverage matrix.

## Coverage

The CI component is `connector-mock-tests` (`scripts/ci/components.py`):
`pytest --cov=connector_tests` produces a Cobertura report checked by the
shared gate (`scripts/ci/coverage.py`, ≥ 80% overall and on new code).
Manifests are YAML — line coverage measures the harness; **behavioral**
coverage of a connector is the spec's stream coverage matrix, enforced per
suite (a skipped matrix row must carry an explicit skip reason).

## Conventions

- Freeze the clock (`freezegun`) in any test touching cursors or
  datetime-templated params.
- Fixtures: response *shapes* from real API payloads, values synthetic — never
  commit real customer data, tokens, or hostnames.
- The `airbyte-cdk` pin must match the `version:` header of the nocode
  manifests (currently the 6.60.x line); bump them in lockstep.
- CDK interpolation literal-evals rendered Jinja values: a numeric-string id in
  `{{ record['id'] }}` becomes an `int` in the emitted record (and in the
  generated schema — cf. `jira_projects.project_id: number`).
