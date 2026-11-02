# The compose-stand suite

Deployed-stand tests for Insight: a real Keycloak login, browser journeys
against the SPA, and an API-contract suite — all run against a local
`docker-compose` stand seeded deterministically for tests
(`src/ingestion/tools/seed`).

This suite assumes an **already-running stand described by a manifest**. API
and persona journeys need the seed fixtures they declare; the portal page smoke
also accepts the minimal, runner-supplied manifest below without a seed. It never
starts compose, applies migrations, or spawns service processes itself —
see `conftest.py`'s module docstring for why that split matters. The shared
library it imports (`insight_stand`, session handling, the manifest reader,
persona resolution) lives in `../lib`; both are one uv project
(`tests/pyproject.toml`).

## Layout

| Path | Contents |
|---|---|
| `conftest.py` | Session wiring: manifest loading, `--stand-manifest`, the `session_for` persona factory, `requires_seed` / capability-marker enforcement. |
| `api/` | HTTP contract tests — no browser. Shared fixtures, the operation catalogue, the scratch policy and the response models live here. |
| `api/analytics/` | The `/api/analytics` prefix, one module per path group. |
| `api/identity/` | The `/api/identity` prefix, one module per concern. |
| `api/test_gateway.py` | Neither service — the edge, sweeping 401 over every catalogued operation at once. |
| `ui/` | The browser journeys, plus `ui/pages/` (page objects). |

Split by service because that is the axis along which a test's setup
differs: identity's answers depend on **who is asking** (the org chart, the
admin row, the kind of principal), analytics' mostly on **what was
created**.

## Running it locally

The stand and the suite are both driven through `./dev-compose.sh
test-stand`, from the repository root:

```bash
./dev-compose.sh test-stand up              # build this tree, bring the stand up, seed it, wait for dbt
./dev-compose.sh test-stand test             # run the whole suite (uv, on the host)
./dev-compose.sh test-stand test tests/stand/api/    # a subset — args pass through to pytest
./dev-compose.sh test-stand down             # stop the stand and drop its volumes
```

`up` compiles the Rust services and builds the SPA from the checkout, so the
suite runs the code you changed. `up --pinned` pulls the published images at
each chart's `appVersion` instead; that is CI's mode, and the fast path when
nothing under `src/backend/` or `src/frontend/` differs from main.

`test-stand test` requires [`uv`](https://docs.astral.sh/uv/) on the host
and runs `uv run --project tests --frozen pytest tests/stand <args>`. Run
pytest directly the same way if you prefer:

```bash
uv sync --project tests
uv run --project tests playwright install chromium   # first time only, for ui/
uv run --project tests pytest tests/stand
```

This host-side run is also exactly what CI's `ui-journeys` lane does. The
verb still accepts `--image <ref>` to run a suite baked into a container
joined to the gateway's network namespace, but no such image is published
anymore — the lane ran from a published `ui-tests` image once, and the mode
remains only as a mechanism. See `dev-compose.sh`'s
`cmd_test_stand_help` for the full verb reference.

## Running it against a deployed stand

The verb is compose-only: it reads `.env.compose.test-stand` and refuses
unless a gateway answers on `http://localhost:<GATEWAY_PORT>/`. Aiming at a
deployed stand means calling pytest directly with the three values that
locate one — where it is, what it was seeded with, and the credential its
personas share:

```bash
export INSIGHT_STAND_BASE_URL=https://<stand host>
export INSIGHT_STAND_PERSONA_PASSWORD=<the realm's persona password>
uv run --project tests --frozen pytest tests/stand -ra \
  --stand-manifest /path/to/stand-manifest.json
```

The manifest is the seed's own record of what it wrote. A completed
`--step all` publishes it to a ConfigMap that outlives the seed Job, so read it
from there:

```bash
kubectl -n <namespace> get configmap seed-stand-manifest \
  -o 'go-template={{index .data "manifest"}}' > stand-manifest.json
```

On a stand seeded before that existed, recover it from the Job log instead —
only while the Job survives its one-hour TTL:

```bash
kubectl -n <namespace> logs job/<the seed job> --tail=-1 \
  | src/ingestion/tools/seed/manifest-from-log.sh \
  | cut -d' ' -f2- > stand-manifest.json
```

It names personas and in-cluster addresses, so it stays on the machine that
read it. Browser journeys need `playwright install chromium` as above; they
work against any `https://` origin, which is trustworthy enough to store the
session cookie.

## Portal page smoke

`ui/test_portal_pages.py` contains 43 catalogue-driven page visits and one
navigation test, marked `stand_smoke` and `reliability`. Each visit opens its
zone, pane entry and optional sub-menu or first list item. Rail navigation
moves the pointer into main and waits until the pane is uncovered.

Four soft checks report together: the expected page test id is visible; main
has no visible loading marker and same-origin fetch/XHR stays idle for one
continuous second within 30 seconds; no Something went wrong alert or
destructive status widget is visible; and the page's requests have no 5xx,
unanswered failure other than `net::ERR_ABORTED`, or Sentry event. Observation
starts immediately before the final hop, after previous requests settle.

The catalogue assigns `ceo` to org-visible pages and `admin_operator` to entries
requiring the identity-service admin grant: Home's My dashboards, the Dashboards
Catalogue entries, Sources & connectors, Identities and Platform usage. Navigation
checks each fixture's supported entries; discoveries are merged across personas
into one report. Rendered page ids exclude aliases and covered dashboard filters;
folder links with a `folder` parameter other than `unfiled` are not visited or
reported. Unresolved page ids remain visible in the report. Me covers the same
person overview as At a glance; Task delivery, Git output, Collaboration and AI
adoption each have a separate person-page test. New actions, Planned / WorkChart
groups and the org tree are excluded.
Usage events are answered locally with 204. `INSIGHT_FORCE_500` accepts a
Playwright URL glob for local 500 responses; when set, Sentry envelopes are
inspected and answered locally too. Visits create or save nothing.

Set `INSIGHT_STAND_BASE_URL` and `INSIGHT_STAND_PERSONA_PASSWORD` as above. Login
adds an empty `kc_idp_hint=` to the authorize redirect before opening the password
form. Browser state is cached in memory by login name, shared between fixture
aliases, and renewed after five minutes. Each test gets a fresh context.
Steps use Allure when installed and are no-ops otherwise; dependencies stay
unchanged. For an Allure report:

```bash
uv run --project tests --frozen --with "allure-pytest>=2.15" \
  pytest tests/stand/ui/test_portal_pages.py \
  --stand-manifest /path/to/stand-manifest.json --alluredir /path/to/allure-results
```

### Minimal page-smoke manifest

This synthetic version-2 example names one person with org visibility and the
active identity admin grant under both fixture aliases. Both entries describe
the same actual realm roles; a seeded manifest instead names separate personas.
`email` carries the login name; unused metadata may stay empty. The smoke uses
neither seed steps nor service URLs. The existing landing journey additionally
requires `dev_lead` when collecting the whole UI directory.

```json
{
  "manifest_version": 2,
  "tenant": "",
  "realm": {"name": "example", "issuer": ""},
  "personas": [],
  "service_urls": {},
  "fixtures": {
    "ceo": {
      "uuid": "", "email": "user@example.com", "display_name": "",
      "team": null, "org_unit": "", "role": "ceo",
      "realm_roles": ["insight-admin", "insight-lead"]
    },
    "admin_operator": {
      "uuid": "", "email": "user@example.com", "display_name": "",
      "team": null, "org_unit": "", "role": "ceo",
      "realm_roles": ["insight-admin", "insight-lead"]
    }
  },
  "capabilities": {
    "idp": "keycloak", "ingestion": false,
    "custom_catalogue": false,
    "files_sharing": false,
    "portal_dashboards": false, "portal_metrics": false,
    "portal_widgets": false, "portal_datasets": false
  },
  "seed_revision": "", "data_window": "", "anchor_date": "", "seeded": []
}
```

Missing requirements skip page visits at collection with a reason and exclude
their navigation entries. The new capabilities default to false. Any `portal_*`
definition capability implies `custom_catalogue`; each promises an existing
definition readable by its catalogued persona. The seed creates none of these
custom definitions. To collect without a stand, supply a synthetic manifest
and `--base-url https://example.com`.

| Capability | Catalogue pages and navigation entries requiring it |
|---|---|
| `custom_catalogue` | Home: My dashboards. Dashboards: All dashboards, Shared with me, Unfiled, Metrics, Widgets, Datasets, Alerts; all first-item visits. |
| `portal_dashboards` | First dashboard (in addition to the service). |
| `portal_metrics` | First metric (in addition to the service). |
| `portal_widgets` | First widget (in addition to the service). |
| `portal_datasets` | First dataset (in addition to the service). |
| `files_sharing` | Explore → Collaboration → Files & sharing; requires its connected data source. |
| `ingestion` | Manage → Sources & connectors. |

## CI required gate

`.github/workflows/e2e-stand.yml` runs the suite in the merge queue. It
starts on every pull request too, with every job skipped, so the required
`api-smoke` and `ui-journeys` contexts exist there and pass as skipped. In the
queue its `changes` job decides whether the diff can affect this suite:
relevant changes run both lanes, irrelevant changes skip them and the lane
checks report success.

## Reading PROFILE.md before writing a test

[`src/ingestion/tools/seed/PROFILE.md`](../../src/ingestion/tools/seed/PROFILE.md) is generated from
the same builder that writes the stand's `manifest.json`, so the two cannot
disagree. Before adding a test, read it for:

- **the roster and fixtures** — the `fixtures{}` table is the set of stable,
  role-shaped names (`dev_lead`, `admin_operator`, …) a test may declare
  against; a raw email or UUID is never a stable target.
- **capabilities** — e.g. `ingestion`, which this stand does not have
  (compose seeds silver/gold directly). A test that needs a capability the
  stand may lack should carry the matching marker (below), not assume it.

What it does not describe is an expected value for any metric: the seed
publishes none, and no test here asserts one. An admissible expectation would
have to be computable from the seed inputs rather than read back out of the
gold layer; reading a number off a running stand and asserting it back only
proves that the code which produced it produced it.

What `api/analytics/test_drilldown.py` does is a different thing and is
allowed: it asks two independent serving relations the same question — the
evidence rows behind a metric, and the metric's own value — and requires them
to agree. Neither side is a number typed into the test, so the seed can change
underneath it, and a disagreement is a real defect rather than a stale
expectation. `drilldown_matrix.py` states what "agree" means per metric.

Regenerate it with `python3 -m insight_seed.render_profile` (from
`src/ingestion/tools/seed`) after changing
the roster or the manifest builder; `--check` verifies it without a
database.

## Declaring required seed data

A test that depends on a specific person or role being present declares it
with `@pytest.mark.requires_seed(*fixture_names)`:

```python
@pytest.mark.requires_seed("dev_lead", "development_ic")
def test_something(session_for):
    lead = session_for("dev_lead")
    ...
```

Every name is checked against the manifest's `fixtures{}` catalog at
**collection time** — before any test runs. A stand seeded without a name a
collected test needs aborts the whole run once, listing every missing name
and every test that needed it, rather than failing tests one at a time as
they happen to run. `requires_ingestion` works the same way for a
capability rather than a person: a test carrying it is **skipped** (not
failed) with a reason, on a stand whose manifest does not declare that
capability.

Both markers are registered in `tests/pyproject.toml`
(`[tool.pytest.ini_options] markers`, with `--strict-markers`) — that is the
one place to look if a new capability marker needs adding.

## UI vs API: the governing rule

**A new UI test must state, in writing, why it cannot instead be an API
test.** A browser is slow, flaky relative to an HTTP call, and exercises
more surface than most assertions need — so the suite would rather have one
more API test than one more browser test whenever the two would prove the
same thing.

State the reason as a paragraph in the test module's docstring, in the
shape the shipped journeys already use — for example
`ui/test_logged_out_access_refused.py`:

> Why this is a browser test and not an API test, measured rather than
> asserted: every SPA route answers 200 text/html to an anonymous HTTP
> client... Refusal exists only inside the browser: the SPA boots, asks
> `/auth/me` (401), and the root route's `beforeLoad` sends the window to
> `/auth/login`...

A justification backed by something you measured (a cookie only a real
browser can set, a client-side route change no HTTP call performs, a
redirect chain that only exists post-render) is what this rule is asking
for — not an assertion that the UI "should" be covered too.

## What this suite does not cover

As of this writing: no metric value is asserted (the golden set is empty by
design, above); no accessibility or contrast checking; cross-tenant
isolation and the service-principal route are left to the in-process
`bronze-to-api` rig.

Four gaps worth naming separately, because none of them is "nobody got to
it":

- **No metrics harness.** There is no suite here that asserts a served metric
  against a declared expectation. One was written and is being migrated
  separately, so this directory has `api/` and `ui/` and nothing else — do not
  read the absence as "metric values are not worth testing".
- **This suite measures its own coverage.** `lib/insight_stand/coverage.py` records
  every response and gates the ledger against the committed OpenAPI document, so a
  route that gains a status code no test exercises is reported. `api/operations.py`
  remains the catalogue of the surface and is kept honest by hand.
- **Cross-tenant refusal.** Covered on compose, and only there: the second
  tenant's caller is a fixture the seed writes when
  `SEED_CROSS_TENANT_FIXTURE` is on, which `docker-compose.yml` sets. A cluster
  stand turns it off (a second tenant aborts identity-resolution's scheduled
  projection), and the seed's manifest then omits `other_tenant_lead`, so tests
  declaring `requires_seed("other_tenant_lead")` skip rather than fail.
- **JWT verification** — an expired token, a wrong audience, an untrusted
  issuer, a signature from a key the JWKS never published. Minting tokens is
  ruled out here by design (see `../lib/insight_stand/session.py`): this suite
  proves the *session*, won by a real login, and the rig proves the token.
  The refusal itself is covered — `api/test_gateway.py` sweeps 401 over every
  operation.
