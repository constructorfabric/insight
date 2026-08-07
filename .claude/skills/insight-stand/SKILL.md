---
name: insight-stand
description: "Operate the Insight compose stand and run the deployed-stand suite in tests/stand/ — bring it up, seed it, aim a run at it (local or remote), read what it was seeded with, and triage the failures that are the stand rather than the product. Use whenever a task means running, re-seeding, pointing, or debugging the stand: 'bring the stand up', 'run the stand tests', 'why did collection abort', 'these tests all skipped', 'the login loops', 'point the suite at another stand', 'what personas does the stand have'. This is the environment skill; stand-api-test and stand-ui-test own the test code, stand-scenarios owns what to test."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

# Working the Insight compose stand

`tests/stand/` tests a **deployed** Insight: real Keycloak login, the gateway
BFF, pinned ghcr images for all four backend services and the frontend, over a
compose stack seeded deterministically by `deploy/seed`.

## The one rule that shapes everything

**The suite never brings the stand up.** `tests/stand/conftest.py` starts no
compose, applies no migrations, spawns no service. It reads an already-running
stand and aborts if it cannot. That split is deliberate — *a run that could
bring its own stand up would hide exactly the deployment failures this suite
exists to catch.*

Two consequences you will meet immediately:

- **The stand must describe itself.** Every fixture name, capability and seeded
  fact comes from `deploy/seed/manifest.json`. Nothing has a default to fall
  back to; a missing or unparseable manifest aborts the session.
- **Unsatisfiable data requirements abort at COLLECTION time**, once, listing
  every missing name — not on test #47.

## Commands

```bash
./dev-compose.sh test-stand up      # generate env, up, seed, block until dbt built gold
./dev-compose.sh test-stand seed    # re-seed a running stand (default target: all)
./dev-compose.sh test-stand test    # run the suite (uv, on the host)
./dev-compose.sh test-stand test tests/stand/api/   # args pass through to pytest, no `--`
./dev-compose.sh test-stand down    # stop AND remove volumes
```

`up` pulls each backend service pinned to its own chart's `appVersion` — never
`:latest`, never compiled here. To test a *backend* change pass
`--build-backend`; `up` otherwise refuses when the tree differs from
`origin/main` under `src/backend/`, since the pinned images would not be what
ran.

`down` removes volumes. There is no lighter reset: **the stand is read-only by
contract and reset by volume teardown, never by TRUNCATE.**

Run pytest directly if you prefer — same thing:

```bash
uv sync --project tests
uv run --project tests playwright install chromium   # first time only, for ui/
uv run --project tests pytest tests/stand
```

Full verb reference: `./dev-compose.sh test-stand --help`.

## Aiming a run

A run is aimed by two independent facts. Get one wrong and the failure looks
like a product bug.

| Fact | Flag | Falls back to |
|---|---|---|
| **Where the stand is** | `--base-url` (pytest-base-url) | `$PYTEST_BASE_URL`, the `base_url` ini key, then `$INSIGHT_STAND_BASE_URL` / the `GATEWAY_PORT` in the stand's own env file |
| **What it was seeded with** | `--stand-manifest <path>` | `$INSIGHT_STAND_MANIFEST`, then `deploy/seed/manifest.json` |

`conftest.pytest_configure` fills pytest-base-url's option only when the
operator has not, so `--base-url` keeps its documented precedence and
everything downstream (the `base_url` fixture, the run header's `baseurl:`
line, `--verify-base-url`, every browser context) behaves as that plugin
documents.

### The base-URL trap — read before pointing a containerised runner

The browser's URL **must be `localhost`-based**, and not for convenience.
`__Host-` prefixed cookies are only stored from a *trustworthy* origin, and
over plain http a browser trusts exactly one host name. Point a runner at
`gateway:8080` and the session cookie is dropped silently: the SPA sees
`/auth/me` 401, restarts the login, and loops until the gateway's rate limiter
turns it into a **503 that looks like a broken backend**.

Chromium's `--unsafely-treat-insecure-origin-as-secure` does not help —
`window.isSecureContext` was measured `false` with the flag on Chromium 149, in
both `launch()` and `launch_persistent_context()`.

So a containerised runner joins the gateway's network namespace and uses
`localhost:<port>`:

```bash
./dev-compose.sh test-stand test --image ghcr.io/constructorfabric/insight-ui-tests:latest
```

That mode never builds — pull first. Test paths become **image-side**
(`/tests/stand/ui`, not `tests/stand/ui`).

## Read the stand before writing against it

[`deploy/seed/PROFILE.md`](../../../deploy/seed/PROFILE.md) is generated from
the same builder that writes `manifest.json`, so the two cannot disagree.
Regenerate with `python3 deploy/seed/render_profile.py`; `--check` verifies it
without a database.

Read it for three things:

- **The `fixtures{}` catalogue** — the stable, role-shaped names a test may
  declare against (`dev_lead`, `development_ic`, `admin_operator`, …). *A raw
  email or UUID is never a stable target.* The names are a contract: they
  describe a role in the org, so renaming one breaks every test that declares
  it.
- **`golden_metrics`** — **empty by design.** No test under `tests/stand/`
  asserts an exact metric value, and none should until the table has entries.
  The admission criterion (`deploy/seed/golden_metrics.py`) is that an
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
| `@pytest.mark.requires_service_principal` | that item **skips**, with a reason | the authenticator's token listener is unreachable from this runner |
| `@pytest.mark.requires_catalogue(*parts)` | that item **skips**, with a reason | rows `deploy/seed/analytics.py` writes are absent |

Two different resolutions on purpose: a missing *fixture* is a defect in how
the stand was prepared; a missing *capability* is a fact about it.

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

The stand persists between runs. A leaked row does not break the run that
leaked it — **it changes what the next run sees**, which is the kind of failure
that gets diagnosed as flakiness.

1. A test may create rows **through the API**, and must delete them. Never a
   database connection: that would hand every test a back door around the
   deployed path.
2. Every created row carries `SCRATCH_PREFIX` (`stand-scratch`) plus a
   per-session `RUN_TAG`, so a leak is identifiable and attributable.
3. **The metric catalog is out of bounds** — it is the metric-coverage gate's
   universe.
4. Teardown deletes are best-effort: a delete-case test already removed its
   row, so a 404 in teardown is the expected outcome.

Rule 2 exists to make rule 1 checkable: the session-scoped
`no_scratch_rows_survive` fixture fails the run if anything survives it.

## Triage — is this the stand or the product?

| Symptom | Cause | Fix |
|---|---|---|
| `UsageError: requires_seed: manifest is missing fixtures…` | stand seeded without those names, or seeded with an older roster | `./dev-compose.sh test-stand seed` |
| `UsageError: stand manifest unusable` | wrong `--stand-manifest`, or `up` never finished seeding | check the path the message quotes |
| `UsageError: the stand endpoint was never resolved` | no address from any of the four sources | `--base-url`, or run from the repo root where the env file lives |
| Everything skips with "capability … not present" | expected on this stand (`ingestion: no`) | not a failure — read the reason |
| Login loops, then 503 | **base URL is not `localhost`** — see the trap above | join the gateway netns, or use the published port |
| `scratch resources survived the run` | a test created rows and did not delete them | find it by the `RUN_TAG` in the leaked names |
| Metric value assertions | there are none, by design | `golden_metrics` is empty — see PROFILE.md |
| Tests pass but assert nothing real | a stand where identity is perfect and the SPA renders an empty shell passes every API test | that is what the UI journeys are for |

Every message about the stand quotes the `source_path` it believed, so a
failure can always name the document behind it.

## Artefacts

At session end the suite writes, **unconditionally** — a failing run's ledger
is the more useful of the two, and making the gate's input depend on the
suite's verdict is backwards:

- `.artifacts/stand_observed_endpoints.json` — the coverage ledger, every
  client in the suite recording into it, browser journeys included.
- `.artifacts/stand_operations.json` — the operation catalogue from
  `tests/stand/api/operations.py`.

The gate (`tests/lib/insight_stand/coverage.py`) compares the two. It is a
stdlib script over two JSON files: runnable on a machine with no stand, no uv
and no browser.

## Hand off

- **What to test** → `stand-scenarios`
- **An API case** → `stand-api-test`
- **A browser journey** → `stand-ui-test`
- **Looking at a UI by hand** → `drive-ui`, `playwright-cli`
- **A real product defect found here** → `file-bug-insight`
