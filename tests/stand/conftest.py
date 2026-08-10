"""Suite-wide wiring for the deployed-stand tests.

This conftest assumes an ALREADY-RUNNING stand. It does not start compose, does
not spawn service processes and does not apply migrations — deliberately unlike
`src/ingestion/tests/e2e/conftest.py`, whose `compose_stack` /
`ch_migrations_applied` / `analytics` fixtures own that lifecycle because that
rig builds its own private stack per session. Here the stand is brought up and
seeded by `./dev-compose.sh test-stand up`, and the suite only reads it. A test
run that could bring its own stand up would hide exactly the deployment failures
this suite exists to catch.

Two rules follow from that split:

* The stand must describe itself. Every fixture name, capability and seeded
  fact comes from `src/ingestion/tools/seed/manifest.json`. If it is missing or unparseable
  the session aborts; nothing here has a default to fall back to.
* Unsatisfiable data requirements are a COLLECTION-time abort, not a run-time
  failure. Finding out on test #47 that the stand was never seeded wastes the
  run and buries the cause.
"""

from __future__ import annotations

import sys
from collections.abc import Callable
from pathlib import Path

import pytest

# `tests/lib` on sys.path so a bare `pytest tests/stand` works in a checkout
# that has not been synced. `uv sync --project tests` installs the same package
# and takes precedence naturally.
_REPO_ROOT = Path(__file__).resolve().parents[2]
_LIB_PATH = _REPO_ROOT / "tests" / "lib"
if str(_LIB_PATH) not in sys.path:
    sys.path.insert(0, str(_LIB_PATH))

from insight_stand import (  # noqa: E402  (import follows the sys.path bootstrap)
    ADMIN_OPERATOR_FIXTURE,
    ADMIN_ROLE,
    LEAD_ROLE,
    MANIFEST_PATH,
    MANIFEST_PATH_ENV,
    MEMBER_ROLE,
    OTHER_TENANT_FIXTURE,
    ApiClient,
    Manifest,
    ManifestError,
    PersonaSession,
    ServiceTokenSession,
    StandConnectionError,
    StandEndpoint,
    artifact_dir,
    coverage,
    default_identity_url,
    default_manifest_path,
    open_service_session,
    open_session,
    resolve_by_realm_role,
    resolve_endpoint,
)

# Marker -> the manifest capability it requires. A test carrying one of these
# is SKIPPED (with a reason) on a stand that lacks the capability, never failed
# and never silently dropped. Add a row to extend; nothing else changes.
CAPABILITY_MARKERS: dict[str, str] = {
    "requires_ingestion": "ingestion",
    "requires_service_principal": "service_principals",
}

#: Where the coverage ledger lands at session end. Read by the gate
#: (`insight_stand/coverage.py`) and uploaded by CI; gitignored like every other
#: run artefact.
ARTIFACT_DIR = ".artifacts"
LEDGER_NAME = "stand_observed_endpoints.json"

# `requires_catalogue(<part>)` -> is that part present? Separate from the
# capability markers above because a catalogue part is DATA the seed wrote, not
# a boolean the stand declares about itself — `has_capability` would have to
# invent a capability name for each one.
CATALOGUE_PARTS: dict[str, Callable[[Manifest], bool]] = {
    "definition_override": lambda m: m.catalogue.definition_override is not None,
}

_MANIFEST: Manifest | None = None
_MANIFEST_PATH: Path | None = None
_ENDPOINT: StandEndpoint | None = None


def _manifest() -> Manifest:
    """Load the manifest once per session, from wherever the run was pointed.

    `pytest_configure` resolves the path; every message about the stand quotes
    the `source_path` that came back, so a suite can always name the document
    it believed.
    """
    global _MANIFEST
    if _MANIFEST is None:
        _MANIFEST = Manifest.load(_MANIFEST_PATH)
    return _MANIFEST


# ---------------------------------------------------------------------------
# Hooks
# ---------------------------------------------------------------------------


# `requires_seed` / `requires_ingestion` are declared in tests/pyproject.toml's
# `[tool.pytest.ini_options] markers`, together with `--strict-markers` and the
# `-ra` that keeps every skip reported with its reason. They are deliberately
# NOT re-registered here: two declarations of the same marker are two places to
# drift, and the project config is the one a reader looks at first.


def pytest_addoption(parser: pytest.Parser) -> None:
    """`--stand-manifest`, for parity with pytest-base-url's `--base-url`.

    A run is aimed by two facts: WHERE the stand is, and WHAT it was seeded
    with. The first already had a command-line flag, contributed by
    pytest-base-url; this gives the second one too, so aiming a run is one
    command line rather than a command line plus an environment variable.

    Deliberately no matching ini key. A default base URL can reasonably be
    pinned in committed config; the path to a particular run's manifest cannot,
    and an ini key would be one more place for a stale answer to hide.
    """
    parser.addoption(
        "--stand-manifest",
        metavar="path",
        default=None,
        help=(
            "seed manifest describing the stand under test "
            f"(default: ${MANIFEST_PATH_ENV}, else {MANIFEST_PATH})"
        ),
    )


def pytest_configure(config: pytest.Config) -> None:
    """Resolve the stand's address into pytest-base-url's own option.

    pytest-base-url — installed as a dependency of pytest-playwright — already
    owns the idea of "the address under test". It offers three ways to set one
    (`--base-url`, the `base_url` ini key, `$PYTEST_BASE_URL`), folds them into
    `config.option.base_url`, and serves the result from a session-scoped
    `base_url` fixture. pytest-playwright reads exactly that fixture to set
    `base_url` on every browser context.

    None of those three can name THIS stand: its address is only known at run
    time, from `$INSIGHT_STAND_BASE_URL` or the `GATEWAY_PORT` in the env file
    the stand itself wrote. The wiring is therefore to fill the plugin's option
    rather than to shadow its fixture — and to fill it only when the operator
    has not, so `--base-url` and friends keep their documented precedence. In
    return everything downstream behaves as the plugin documents it: the
    `base_url` fixture, the `baseurl:` line in the run header, and
    `--verify-base-url`.


    """
    global _ENDPOINT, _MANIFEST_PATH

    chosen = config.getoption("--stand-manifest")
    _MANIFEST_PATH = Path(str(chosen)) if chosen else default_manifest_path()

    # Both sources are read here, exactly as pytest-base-url's own
    # `pytest_configure` reads them, because that hook has not run yet: pytest
    # calls conftest hooks BEFORE entry-point plugin hooks, so by the time the
    # plugin folds its ini key into the option this has already filled it.
    # Checking only the option would silently ignore a `base_url` ini setting.
    configured = config.getoption("base_url", default=None) or config.getini("base_url")
    if configured:
        _ENDPOINT = StandEndpoint(
            base_url=str(configured).rstrip("/"),
            source="--base-url / $PYTEST_BASE_URL / base_url ini",
        )
    else:
        try:
            _ENDPOINT = resolve_endpoint()
        except StandConnectionError as exc:
            # Same "refuse to start" contract as an unusable manifest below,
            # just earlier: a suite that cannot say where the stand is has
            # nothing to test.
            raise pytest.UsageError(str(exc)) from exc
    config.option.base_url = _ENDPOINT.base_url


def pytest_sessionfinish(session: pytest.Session, exitstatus: int) -> None:
    """Write the coverage ledger, whatever the run's verdict was.

    Unconditionally on purpose. A failing run's ledger is the more useful of the
    two — it says which operations were reached before things went wrong — and
    writing only on success would make the gate's input depend on the suite's
    result, which is backwards.

    Every client in the suite records here, browser journeys included — they
    drive `ApiClient` for their setup — so this is the whole run's ledger, not
    the api directory's. `api/conftest.py` writes the operation CATALOGUE beside
    it, because that list is the api package's.
    """
    del session, exitstatus

    coverage.dump(artifact_dir(_REPO_ROOT / ARTIFACT_DIR) / LEDGER_NAME)


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    """Validate data requirements before a single test runs.

    Two different resolutions, on purpose:

    * `requires_seed` — a missing fixture means the stand was seeded wrong.
      Every collected test is inspected, every missing name is gathered, and
      the session aborts ONCE with all of them listed. Failing per-test would
      report the same root cause dozens of times; failing on the first miss
      would hide the rest and force a fix-rerun-discover loop.
    * capability markers — a missing capability is a legitimate property of
      this stand, not a defect, so it skips that item alone.
    * `requires_catalogue` — same resolution as a capability, for the rows
      `src/ingestion/tools/seed/analytics.py` writes. A stand seeded without that step is a
      real state, and a test that needs those rows must say so rather than
      assert against an empty universe and pass for the wrong reason.
    """
    del config

    try:
        manifest = _manifest()
    except ManifestError as exc:
        # UsageError aborts the session with a non-zero exit before any test
        # runs — the "refuse to start" contract.
        raise pytest.UsageError(f"stand manifest unusable: {exc}") from exc

    missing: dict[str, list[str]] = {}
    for item in items:
        for marker in item.iter_markers(name="requires_seed"):
            for name in marker.args:
                if name not in manifest.seeded_names:
                    missing.setdefault(str(name), []).append(item.nodeid)

    if missing:
        lines = [
            f"  - {name!r} required by: {', '.join(nodeids)}"
            for name, nodeids in sorted(missing.items())
        ]
        available = ", ".join(sorted(manifest.seeded_names)) or "<none>"
        raise pytest.UsageError(
            "requires_seed: manifest is missing fixtures needed by collected tests:\n"
            + "\n".join(lines)
            + f"\n  manifest: {manifest.source_path} (seeded steps: "
            + f"{', '.join(manifest.seeded) or 'none'})"
            + f"\n  available fixtures: {available}"
            + "\n  Re-seed the stand:  ./dev-compose.sh test-stand seed"
        )

    for item in items:
        for marker in item.iter_markers(name="requires_catalogue"):
            for part in marker.args:
                if part not in CATALOGUE_PARTS:
                    raise pytest.UsageError(
                        f"requires_catalogue({part!r}) on {item.nodeid}: unknown part. "
                        f"Supported: {', '.join(sorted(CATALOGUE_PARTS))}"
                    )
                if CATALOGUE_PARTS[part](manifest):
                    continue
                item.add_marker(
                    pytest.mark.skip(
                        reason=(
                            f"requires_catalogue: no {part!r} in the manifest's catalogue "
                            f"({manifest.source_path}) — re-seed with the analytics step: "
                            "./dev-compose.sh test-stand seed all"
                        )
                    )
                )

    for item in items:
        for marker_name, capability in CAPABILITY_MARKERS.items():
            if item.get_closest_marker(marker_name) is None:
                continue
            try:
                satisfied = manifest.has_capability(capability)
            except ValueError as exc:
                # A typo in CAPABILITY_MARKERS above, not a property of the
                # stand. Left unchecked it would skip every test carrying the
                # marker with a reason that reads perfectly plausibly.
                raise pytest.UsageError(
                    f"CAPABILITY_MARKERS maps {marker_name!r} to an unknown capability: {exc}"
                ) from exc
            if satisfied:
                continue
            item.add_marker(
                pytest.mark.skip(
                    reason=(
                        f"{marker_name}: manifest capability {capability!r} not "
                        f"present on this stand ({manifest.source_path})"
                    )
                )
            )


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def stand_manifest() -> Manifest:
    """The stand's self-description. Raises rather than defaulting."""
    return _manifest()


@pytest.fixture(scope="session")
def stand_endpoint() -> StandEndpoint:
    """Where the stand is, plus which file or variable said so.

    The `source` half is why this exists alongside `base_url`: a connection
    failure can then name the file or variable that produced the address which
    did not answer, instead of only that it did not.
    """
    if _ENDPOINT is None:  # pragma: no cover - pytest_configure always runs
        raise pytest.UsageError("the stand endpoint was never resolved")
    return _ENDPOINT


@pytest.fixture(scope="session")
def stand_base_url(base_url: str) -> str:
    """The stand's address, taken from pytest-base-url.

    Deliberately NOT resolved again here. This is the same fixture
    pytest-playwright reads to configure every browser context, so an API
    client built from it is addressing the stand the browser is driving by
    construction rather than by coincidence.
    """
    return base_url


@pytest.fixture
def api_client(stand_base_url: str) -> ApiClient:
    """Gateway-fronted client with NO session — genuinely unauthenticated.

    For an authenticated client, take `.client` off a `PersonaSession` from
    `session_for` below.
    """
    return ApiClient(base_url=stand_base_url)


# ---------------------------------------------------------------------------
# Person fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def session_for(stand_manifest: Manifest, stand_base_url: str) -> Callable[[str], PersonaSession]:
    """`session_for("dev_lead")` → that persona's real, verified session.



    The argument is a key in the manifest's `fixtures{}` catalog, never an
    email and never a UUID, so a roster reshuffle moves the person without
    touching a single test. Unknown names fail naming what is available.

    Every session is won by driving the deployed OIDC chain against Keycloak:
    `/auth/login` → the real HTML login form → `/auth/callback` → `__Host-sid`.
    Nothing here mints a token; that is the in-process rig's path, and using it
    would mean this suite never exercises the login it exists to test.

    Cached sessions re-acquire themselves before the stand's 10-minute TTL can
    expire mid-suite.
    """
    cache: dict[str, PersonaSession] = {}

    def _session_for(name: str) -> PersonaSession:
        if name not in cache:
            cache[name] = open_session(name, stand_manifest, stand_base_url)
        return cache[name]

    return _session_for


@pytest.fixture(scope="session")
def realm_admin_session(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> PersonaSession:
    """An ORG MEMBER the realm granted `insight-admin` — in practice the CEO.

    Named for the realm role on purpose, because that is all it is. No identity
    endpoint reads `insight-admin`: the admin gate consults an active `admin`
    row in `identity.person_roles` and this persona has none. Use it where the
    point is a senior person's view of the organisation; use
    `admin_operator_session` where the point is administrative authority.

    Not `admin_session` — that name is what confuses it with the fixture below.
    """
    return session_for(resolve_by_realm_role(stand_manifest, ADMIN_ROLE))


@pytest.fixture(scope="session")
def admin_operator_session(
    session_for: Callable[[str], PersonaSession],
) -> PersonaSession:
    """The account that actually opens the admin-gated identity API.

    It holds the `admin` row in `identity.person_roles`, which `require_admin`
    resolves from the gateway JWT — the only thing that gate looks at.

    Deliberately outside the org chart: no team, no edge in either direction. So
    it contributes no activity data, sees nobody in `/v1/subchart`, and cannot
    perturb a visibility assertion. That isolation is the reason it is a separate
    person rather than a grant bolted onto the CEO.
    """
    return session_for(ADMIN_OPERATOR_FIXTURE)


@pytest.fixture(scope="session")
def service_session(stand_manifest: Manifest) -> ServiceTokenSession:
    """A service principal, obtained at the authenticator's token endpoint.

    Not minted. The suite signs an RFC 7523 assertion with the `testclient` key
    the stand generated and exchanges it for a real gateway JWT — so what a test
    carries is a credential the product issued, and the issuance path is
    exercised rather than assumed.

    Session-scoped and self-renewing: the issued token outlives most suites, and
    `headers()` re-exchanges before it expires.
    """
    return open_service_session(stand_manifest.tenant)


@pytest.fixture
def service_client(service_session: ServiceTokenSession) -> ApiClient:
    """A client addressing identity-resolution DIRECTLY, carrying that principal.

    The one client in this suite that does not go through the gateway, and the
    product is why: the edge is a browser BFF that delegates authz to the
    authenticator, which looks for a session cookie and refuses a bearer-only
    request with `401 no_session`. A service principal therefore has no edge
    address — real callers reach `/internal/*` in-network, and so does this.

    Narrow on purpose. It carries `edge_fronted=False`, so `_checked_path` stops
    catching backend-port mistakes for requests made with it; nothing but the
    service-only routes should use it. The human-refusal half of that contract
    stays at `/api/identity/...`, where it belongs.
    """
    return ApiClient(base_url=default_identity_url(), session=service_session, edge_fronted=False)


@pytest.fixture(scope="session")
def other_tenant_session(
    session_for: Callable[[str], PersonaSession],
) -> PersonaSession:
    """A real login as somebody in a DIFFERENT tenant.

    Not a forged token and not a header override: the same Keycloak login every
    other persona uses, differing only in the `tenant_id` the realm carries for
    that user. What it proves is therefore the deployed path — a caller the
    product should refuse, refused for the reason it would be in a deployment.

    A test using this must declare `requires_seed("other_tenant_lead")`, so a
    stand seeded without them aborts naming the missing fixture rather than
    failing at login with something less obvious.
    """
    return session_for(OTHER_TENANT_FIXTURE)


@pytest.fixture(scope="session")
def lead_session(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> PersonaSession:
    """A session for a persona granted `insight-lead` but NOT `insight-admin`.

    Excluding admins matters: the CEO holds both, so without it `lead_session`
    and `realm_admin_session` could resolve to the same person and every
    lead-vs-admin comparison would pass vacuously.
    """
    return session_for(resolve_by_realm_role(stand_manifest, LEAD_ROLE, excluding=ADMIN_ROLE))


@pytest.fixture(scope="session")
def member_session(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> PersonaSession:
    """A session for a persona the realm granted only `insight-member`."""
    return session_for(resolve_by_realm_role(stand_manifest, MEMBER_ROLE))
