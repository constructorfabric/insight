---
name: insight-stand
description: "Operate the Insight compose stand and run the deployed-stand suite in tests/stand/ — bring it up (`./dev-compose.sh test-stand up`), seed it, aim a run at it (local or remote), read what it was seeded with, and triage the failures that are the stand rather than the product. Use whenever a task means running, re-seeding, pointing, or debugging the stand: 'bring the stand up', 'run the stand tests', 'test-stand up failed', 'why did collection abort', 'UsageError: stand manifest unusable', 'these tests all skipped', 'the login loops', 'scratch rows survived the run', 'point the suite at another stand', 'what personas does the stand have'. This is the environment skill; stand-api-test and stand-ui-test own the test code, stand-scenarios owns what to test, and drive-ui owns driving a browser at a stand by hand."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

# Working the Insight compose stand

`tests/stand/` tests a **deployed** Insight: real Keycloak login, the gateway
BFF, pinned ghcr images for all four backend services and the frontend, over a
compose stack seeded deterministically by `src/ingestion/tools/seed`.

## The one rule that shapes everything

**The suite never brings the stand up.** `tests/stand/conftest.py` starts no
compose, applies no migrations, spawns no service. It reads an already-running
stand and aborts if it cannot. That split is deliberate — *a run that could
bring its own stand up would hide exactly the deployment failures this suite
exists to catch.*

Two consequences you will meet immediately:

- **The stand must describe itself.** Every fixture name, capability and seeded
  fact comes from `src/ingestion/tools/seed/manifest.json`. Nothing has a default to fall
  back to; a missing or unparseable manifest aborts the session.
- **Unsatisfiable data requirements abort at COLLECTION time**, once, listing
  every missing name — not on test #47.

## Commands

```bash
./dev-compose.sh test-stand up      # generate env, up, seed, block until dbt built gold
./dev-compose.sh test-stand seed    # re-seed a running stand (default target: all)
./dev-compose.sh test-stand test    # run the suite (uv, on the host)
./dev-compose.sh test-stand test -k subchart        # args pass through to pytest, no `--`
./dev-compose.sh test-stand down    # stop AND remove volumes
```

`up` pulls each backend service pinned to its own chart's `appVersion` — never
`:latest`, never compiled here. To test a *backend* change pass
`--build-backend`; `up` otherwise refuses when the tree differs from
`origin/main` under `src/backend/`, since the pinned images would not be what
ran.

**A path argument does not narrow the run.** The verb appends it to a hardcoded
`tests/stand` and pytest unions path arguments, so `test-stand test
tests/stand/api/` runs the whole suite, browsers included. Use `-k` / `--ignore`,
or run pytest directly against the path. (`--image` mode is the exception: there
the arguments replace the image's own `CMD`.)

`down` removes volumes. There is no lighter reset: **the stand is read-only by
contract and reset by volume teardown, never by TRUNCATE.**

Run pytest directly when you need to — and note `--frozen`, which the verb also
passes. It runs exactly the locked dependency set rather than re-resolving
silently, so the host runner and the `ui-tests` image stay identical:

```bash
uv sync --project tests
uv run --project tests --frozen playwright install chromium   # first time only, for ui/
uv run --project tests --frozen pytest tests/stand
```

Verb reference: `./dev-compose.sh test-stand --help`. It omits `up --auth
<keycloak|fakeidp>`, which exists — read `cmd_test_stand` if you need the
full set.

## Aiming a run

A run is aimed by two independent facts. Get one wrong and the failure looks
like a product bug.

| Fact | Flag | Falls back to |
|---|---|---|
| **Where the stand is** | `--base-url` (pytest-base-url) | `$PYTEST_BASE_URL`, the `base_url` ini key, `$INSIGHT_STAND_BASE_URL`, then the `GATEWAY_PORT` in an env file — `$INSIGHT_STAND_ENV_FILE` if set, else `.env.compose.test-stand`, **else `.env.compose`** |
| **What it was seeded with** | `--stand-manifest <path>` | `$INSIGHT_STAND_MANIFEST`, then `src/ingestion/tools/seed/manifest.json` |

That last fallback is worth knowing: with no test-stand env file present, a run
silently inherits a developer's own `.env.compose` — the exact mis-aim the rest
of this section exists to prevent.

`conftest.pytest_configure` fills pytest-base-url's option only when the
operator has not, so `--base-url` keeps its documented precedence and
everything downstream (the `base_url` fixture, the run header's `baseurl:`
line, `--verify-base-url`, every browser context) behaves as that plugin
documents.

### Aiming at a stand that is not the local one

**Not through `test-stand test`.** That verb reads `GATEWAY_PORT` from the
stand's env file and curl-preflights `http://localhost:<port>/` *before* it
looks at any pytest argument, so it aborts with "gateway is not answering …
Bring the stand up first" no matter what `--base-url` you pass. The message
sends you to bring a stand up, which is the wrong move and an easy loop to get
stuck in.

Aim a run elsewhere by running pytest directly, or by exporting the variable:

```bash
uv run --project tests --frozen pytest tests/stand \
  --base-url https://<stand> --stand-manifest <path>/manifest.json
```

### The base-URL trap — read before pointing a containerised runner

The browser's URL **must be a trustworthy origin**, and not for convenience.
`__Host-` prefixed cookies are only stored from one, and over plain http a
browser trusts exactly one host name: `localhost`. An `https://` stand is
trustworthy already — the constraint is TLS-or-localhost, not localhost as
such. Point a plain-http runner at `gateway:8080` and the session cookie is
dropped silently: the SPA sees
`/auth/me` 401, restarts the login, and loops until the gateway's rate limiter
turns it into a **503 that looks like a broken backend**.

Chromium's `--unsafely-treat-insecure-origin-as-secure` does not help —
`window.isSecureContext` was measured `false` with the flag on Chromium 149, in
both `launch()` and `launch_persistent_context()`.

So a containerised runner joins the gateway's network namespace and uses
`localhost:<port>` (no suite image is published anymore — CI runs host-side
from the checkout, and host `localhost:<port>` satisfies the same
constraint; `--image` remains for a locally built runner):

```bash
./dev-compose.sh test-stand test --image <locally-built-suite-image>
```

Three preconditions, each refused up front rather than failing opaquely later:

- **`GATEWAY_PORT` must be the container port (8080).** The realm registered
  `http://localhost:${GATEWAY_PORT}/auth/callback` while an in-namespace
  browser reaches the gateway at its *container* port; when they differ the
  login dies several steps later as an opaque IdP error.
- `src/ingestion/tools/seed/manifest.json` must exist — seed first.
- The image must already be pulled. This mode never builds it.

Test paths become **image-side** (`/tests/stand/ui`, not `tests/stand/ui`).

## Read the stand before writing against it

[`src/ingestion/tools/seed/PROFILE.md`](../../../src/ingestion/tools/seed/PROFILE.md) is generated from
the same builder that writes `manifest.json`, so the two cannot disagree.
Regenerate with `python3 -m insight_seed.render_profile`; `--check` verifies it
without a database.

Read it for three things:

- **The `fixtures{}` catalogue** — the stable, role-shaped names a test may
  declare against (`dev_lead`, `development_ic`, `admin_operator`, …). *A raw
  email or UUID is never a stable target.* The names are a contract: they
  describe a role in the org, so renaming one breaks every test that declares
  it.
- **`golden_metrics`** — **empty by design.** No test under `tests/stand/`
  asserts an exact metric value, and none should until the table has entries.
  The admission criterion (`src/ingestion/tools/seed/insight_seed/golden_metrics.py`) is that an
  expectation must be *computable from the seed inputs*, not read back out of
  the gold layer. Reading a number off a running stand and asserting it back
  only proves that the code which produced it produced it.
- **`capabilities`** — e.g. `ingestion: no` (compose seeds silver/gold
  directly, so no connector runs). A test needing a capability the stand may
  lack carries the matching marker rather than assuming.

## Declaring what a test needs

| Marker | Resolution when unmet | Meaning |
|---|---|---|
| `@pytest.mark.requires_seed(*names)` | **session aborts** at collection, listing every missing name and every test that needed it | the stand was seeded wrong |
| `@pytest.mark.requires_ingestion` | that item **skips**, with a reason | a legitimate property of this stand |
| `@pytest.mark.requires_service_principal` | that item **skips**, with a reason | the stand's manifest does not declare `service_principals` — i.e. its token listener is not published. Read the manifest, not your container |
| `@pytest.mark.requires_catalogue(*parts)` | that item **skips**, with a reason | rows `src/ingestion/tools/seed/insight_seed/analytics.py` writes are absent |

Two different resolutions on purpose: a missing *fixture* is a defect in how
the stand was prepared; a missing *capability* is a fact about it.

Only two are carried by shipped tests today — `requires_seed` and
`requires_service_principal`. `requires_ingestion` and `requires_catalogue` are
registered and unused, so do not go looking for an example of them; they are
the extension contract, not current practice.

All four are registered in `tests/pyproject.toml` under
`[tool.pytest.ini_options] markers`, with `--strict-markers` and `-ra` — that
is the single place to add one. Do not re-register them in a conftest.

Extending: a new capability marker needs a row in `CAPABILITY_MARKERS`
(`tests/stand/conftest.py`); a new catalogue part needs one in
`CATALOGUE_PARTS`. A typo in either is caught as a `UsageError`, not silently
skipped — that check exists because a wrong mapping would skip every test
carrying the marker with a reason that reads perfectly plausibly.

## Sessions — who a request is

`session_for("dev_lead")` returns that persona's **real, verified session**,
won by driving the deployed OIDC chain: `/auth/login` → Keycloak's real HTML
form → `/auth/callback` → `__Host-sid`. **Nothing here mints a token** — that
is the in-process rig's path, and using it would mean the suite never exercises
the login it exists to test. Sessions are cached and re-acquire before the
stand's 10-minute TTL.

| Fixture | Who | Use it when |
|---|---|---|
| `session_for(name)` | any fixture name from the manifest | you need a *specific* role in the org |
| `lead_session` | `insight-lead` but **not** `insight-admin` | the ordinary authenticated caller |
| `realm_admin_session` | an org member the realm granted `insight-admin` (the CEO) | a senior person's **view of the organisation** |
| `admin_operator_session` | holds the `admin` row in `identity.person_roles` | **administrative authority** |
| `other_tenant_session` | a real login in a different tenant | cross-tenant refusal |
| `member_session` | `insight-member` only | the narrowest principal |
| `service_client` | RFC 7523 assertion exchanged for a real gateway JWT | `/internal/*`, direct to identity, **not** gateway-fronted |
| `api_client` | no session at all | genuinely unauthenticated |

**The distinction that catches people:** `realm_admin_session` ≠
`admin_operator_session`. No identity endpoint reads the `insight-admin` *realm
role* — `require_admin` consults an active `admin` row in
`identity.person_roles`, and the CEO has none. `lead_session` explicitly
excludes admins so lead-vs-admin comparisons cannot pass vacuously (the CEO
holds both realm roles).

`admin_operator` is deliberately **outside the org chart**: no team, no edge in
either direction, so it contributes no activity and sees nobody in
`/v1/subchart`. That isolation is why it is a separate person rather than a
grant bolted onto the CEO.

## Mutation policy

The stand persists between runs and is reset by volume teardown, never by
TRUNCATE. A leaked row does not break the run that leaked it — **it changes
what the next run sees**, which is the kind of failure that gets diagnosed as
flakiness. The session-scoped `no_scratch_rows_survive` fixture fails a run
that leaks one.

The rules themselves live with the code that implements them
(`tests/stand/api/scratch.py`) and in `stand-api-test`, which owns writing
tests that create rows. What you need here is the triage row below.

## Triage — is this the stand or the product?

| Symptom | Cause | Fix |
|---|---|---|
| `UsageError: requires_seed: manifest is missing fixtures…` | stand seeded without those names, or seeded with an older roster | `./dev-compose.sh test-stand seed` |
| `UsageError: stand manifest unusable` | wrong `--stand-manifest`, or `up` never finished seeding | check the path the message quotes |
| `cannot resolve the stand's base URL — refusing to assume one` (lists what it tried) | no address from any source | `--base-url`, or run from the repo root where the env file lives |
| `gateway is not answering on http://localhost:<port>/` | you aimed `test-stand test` at a stand that is not local | the wrapper preflights localhost — run pytest directly instead |
| Everything skips with "capability … not present" | expected on this stand (`ingestion: no`) | not a failure — read the reason |
| Login loops, then 503 | **base URL is not `localhost`** — see the trap above | join the gateway netns, or use the published port |
| `scratch resources survived the run` | a test created rows and did not delete them | find it by the `RUN_TAG` in the leaked names |
| Metric value assertions | there are none, by design | `golden_metrics` is empty — see PROFILE.md |
| Tests pass but assert nothing real | a stand where identity is perfect and the SPA renders an empty shell passes every API test | that is what the UI journeys are for |

Every message about the stand quotes the `source_path` it believed, so a
failure can always name the document behind it.

## Artefacts

Both are written at session end regardless of the run's verdict — a failing
run's ledger is the more useful of the two, and making the gate's input depend
on the suite's result is backwards.

- `.artifacts/stand_observed_endpoints.json` — the coverage ledger. Recording
  happens in `ApiClient.request`, not at construction, so this is the **API
  suite's** ledger: a browser journey holds a `PersonaSession` — which does
  carry a client — but never issues a request through it, so a ui-only run
  contributes nothing.
- `.artifacts/stand_operations.json` — the operation catalogue, written by
  `api/conftest.py`. A ui-only run therefore produces no catalogue at all, and
  the gate requires one.

The gate compares the two. It is stdlib-only, so it runs on a machine with no
stand, no uv and no browser:

```bash
python3 tests/lib/insight_stand/coverage.py \
  --observed .artifacts/stand_observed_endpoints.json \
  --catalogue .artifacts/stand_operations.json
```

## Hand off

- **What to test** → `stand-scenarios`
- **An API case** → `stand-api-test`
- **A browser journey** → `stand-ui-test`
- **Looking at a UI by hand** → `drive-ui`, `playwright-cli`
- **Designing what to cover** → `stand-scenario-designer` (agent)
- **Checking a test proves its claim** → `stand-test-auditor` (agent)
- **A real product defect found here** → `file-bug-insight`
