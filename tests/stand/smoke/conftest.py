"""Wiring for the deployed-stand smoke: aiming it, and logging each persona in once.

Three jobs, and the first one is the one that needs explaining.


1. Aiming the run
-----------------

Every other directory under `tests/stand/` targets the local compose stand, and
`tests/stand/conftest.py` resolves that address from `$INSIGHT_STAND_BASE_URL`
or the `GATEWAY_PORT` in `.env.compose.test-stand`. This directory targets a
DEPLOYED stand through its public URL, named by `$SMOKE_BASE_URL` and by nothing
else.

That creates one problem worth stating plainly: `tests/stand/smoke/` is
collected by anything that collects `tests/stand/`, including the existing
compose lane in `.github/workflows/e2e-stand.yml`, which narrows only with
`--ignore=tests/stand/ui`. A deploy gate that hard-failed there would turn a
lane red for a suite it was never aimed at. So the rule is:

* the command line NAMES this directory and `$SMOKE_BASE_URL` is unset
  → `pytest.UsageError`. You asked for the smoke; you forgot to aim it.
* the directory was merely swept up by a broader collection and
  `$SMOKE_BASE_URL` is unset → every check SKIPS, with a reason naming the
  variable. `-ra` is in `addopts`, so the reason is printed; it is not silent.
* `$SMOKE_BASE_URL` is set → everything else required is mandatory, and a
  missing value aborts the session naming the variable. Nothing after this
  point skips, and in particular a login that cannot work FAILS, with the cause
  diagnosed from the step it stopped at (see `login.py`'s
  `describe_login_failure`).

When this conftest is an INITIAL conftest — which it is whenever the command
line names a path inside this directory — its `pytest_configure` runs before the
parent's. That is what lets it hand `$SMOKE_BASE_URL` to the parent's stand
resolution, so a smoke-only run against a cluster does not abort in a fixture
that was looking for a compose env file. Verified behaviour, not an assumption:
pytest loads initial conftests during pre-parse and calls conftest hooks
deepest-first.


2. Choosing the personas
------------------------

D13 wants MULTIPLE personas, and the manifest's `fixtures{}` catalog is the only
canonical source of who exists — `PROFILE.md` is generated documentation and no
code reads it. The roster is therefore resolved BY REALM ROLE
(`resolve_by_realm_role`) rather than by hardcoded fixture names, so it keeps
meaning "one person at each authority level this stand actually seeded" through
a roster reshuffle.

The admin OPERATOR is included when the stand seeded one, because it is the
account an operator would log in with and it authenticates through a different
row than the org roles do. It is deliberately excluded from the metric probe:
it sits outside the org chart, so it has no activity of its own.


3. Logging in once per persona
------------------------------

`smoke_login` is a session-scoped FACTORY with its own cache, mirroring
`session_for` in the parent conftest. A session-scoped fixture cannot depend on
a function-scoped parameter, and a function-scoped login fixture would
re-authenticate for every check — which on a rate-limited public stand is a
self-inflicted failure.
"""

from __future__ import annotations

import os
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import Final

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    ADMIN_ROLE,
    LEAD_ROLE,
    MEMBER_ROLE,
    Manifest,
    ManifestError,
    PersonaError,
    default_manifest_path,
    resolve_by_realm_role,
)
from insight_stand import (
    BASE_URL_ENV as STAND_BASE_URL_ENV,
)

from .login import (
    BASE_URL_ENV,
    SmokeCredentials,
    SmokeLogin,
    open_smoke_session,
    resolve_credentials,
)

#: This directory, as it appears on a pytest command line. Used only to answer
#: "did the operator ask for the smoke, or merely collect it".
_SMOKE_DIR: Final[str] = "tests/stand/smoke"

#: Role label -> how to find that persona in the manifest. Ordered, because the
#: parametrization ids and the login order follow it, and a deploy gate reads
#: better when it walks authority from the top down.
#:
#: `admin` resolves to an ORG MEMBER holding `insight-admin` (in practice the
#: CEO), never the operator — `resolve_by_realm_role` skips operator accounts
#: for exactly that reason. `lead` excludes admins so the two rows cannot
#: collapse onto the same person and make the roster one persona shorter than it
#: claims to be.
_ROLE_LOOKUPS: Final[tuple[tuple[str, str, str | None], ...]] = (
    ("admin", ADMIN_ROLE, None),
    ("lead", LEAD_ROLE, ADMIN_ROLE),
    ("member", MEMBER_ROLE, None),
)

#: The role label the metric probe asks its question as. A lead has both their
#: own seeded activity and a subtree, so a 403 from the visibility gate would be
#: a real defect rather than an artefact of asking as the wrong person.
METRIC_PROBE_ROLE: Final[str] = "lead"

_MANIFEST: Manifest | None = None
_CREDENTIALS: SmokeCredentials | None = None


# ---------------------------------------------------------------------------
# Aiming
# ---------------------------------------------------------------------------


def _aimed() -> bool:
    return bool((os.environ.get(BASE_URL_ENV) or "").strip())


def _explicitly_selected(config: pytest.Config) -> bool:
    """Did the command line name a path inside this directory?

    `config.invocation_params.args` is the raw argument list, before pytest
    normalises anything, which is what makes it the right thing to read: a
    `tests/stand` sweep and a `tests/stand/smoke` request are two different
    intents and only the raw arguments still tell them apart.
    """
    return any(
        _SMOKE_DIR in str(argument).replace(os.sep, "/")
        for argument in config.invocation_params.args
    )


def _not_aimed_reason() -> str:
    return (
        f"${BASE_URL_ENV} is not set, so this run is not aimed at a deployed stand. "
        "These checks address a cluster stand through its public URL and are skipped "
        "when a broader collection sweeps them up (see tests/stand/smoke/README.md)."
    )


def pytest_configure(config: pytest.Config) -> None:
    """Refuse an unaimed explicit request, and hand the address to the parent.

    The alias is the whole reason this hook exists. `tests/stand/conftest.py`
    resolves the stand's address in ITS `pytest_configure` and raises
    `pytest.UsageError` when it cannot — correct for a compose run, and fatal
    for a smoke-only run whose address lives in a variable that conftest has
    never heard of. Copying the value across is a smaller, more honest fix than
    teaching the shared conftest about a directory-specific variable, and it
    only ever fires when nothing else has aimed the run.
    """
    if _explicitly_selected(config) and not _aimed():
        raise pytest.UsageError(
            f"${BASE_URL_ENV} is not set, but the command line asked for {_SMOKE_DIR}.\n"
            "  It is the deployed stand's public address, e.g. "
            f"{BASE_URL_ENV}=https://<stand-host> pytest {_SMOKE_DIR}\n"
            "  Every variable this suite reads is listed in tests/stand/smoke/README.md; "
            "none of them has a default."
        )

    aimed_at = (os.environ.get(BASE_URL_ENV) or "").strip()
    already_aimed = (
        config.getoption("base_url", default=None)
        or config.getini("base_url")
        or (os.environ.get(STAND_BASE_URL_ENV) or "").strip()
    )
    if aimed_at and not already_aimed:
        os.environ[STAND_BASE_URL_ENV] = aimed_at.rstrip("/")


# ---------------------------------------------------------------------------
# The roster, resolved at collection time
# ---------------------------------------------------------------------------


def _manifest(config: pytest.Config) -> Manifest:
    """The stand's self-description, loaded the way the parent conftest loads it.

    Loaded again here rather than borrowed, because the parent keeps its copy in
    a module-private global and the parametrization below needs the roster
    BEFORE any fixture can run. Both readers are read-only and both honour
    `--stand-manifest` then `$INSIGHT_STAND_MANIFEST`, so they cannot disagree
    about which document describes the stand.
    """
    global _MANIFEST
    if _MANIFEST is None:
        chosen = config.getoption("--stand-manifest")
        _MANIFEST = Manifest.load(Path(str(chosen)) if chosen else default_manifest_path())
    return _MANIFEST


def smoke_roster(manifest: Manifest) -> Mapping[str, str]:
    """Role label -> manifest fixture name, for the personas this suite drives.

    Raises `PersonaError` when a stand seeded no persona at some authority
    level. That is the right disposition: a deploy gate that quietly tested one
    persona because the other two could not be found would still be green while
    proving a third of what it claims.
    """
    roster = {
        label: resolve_by_realm_role(manifest, role, excluding=excluding)
        for label, role, excluding in _ROLE_LOOKUPS
    }
    # Present on any stand the seeder wrote; absent only on a stand seeded by
    # something else, in which case there is nothing to log in as and nothing to
    # report — the org roles above already cover D13's "multiple personas".
    if ADMIN_OPERATOR_FIXTURE in manifest.seeded_names:
        roster["operator"] = ADMIN_OPERATOR_FIXTURE
    return roster


def pytest_generate_tests(metafunc: pytest.Metafunc) -> None:
    """Parametrize the per-persona checks over the roster this stand actually has.

    The ids are the manifest fixture names rather than the role labels, so a
    failing check names the person a reader can look up in the manifest.
    """
    if "persona_name" not in metafunc.fixturenames:
        return

    try:
        names = tuple(smoke_roster(_manifest(metafunc.config)).values())
    except (ManifestError, PersonaError) as exc:
        if not _aimed():
            # Not our stand and not our problem: the checks are about to be
            # skipped anyway, and a placeholder keeps that skip visible instead
            # of turning an unaimed sweep into a collection error.
            metafunc.parametrize("persona_name", ["<unresolved>"], ids=["unresolved"])
            return
        raise pytest.UsageError(
            f"cannot choose the smoke personas from the stand's manifest: {exc}\n"
            "  The roster is resolved by realm role, so this means the manifest "
            "describes no persona at one of the levels this suite drives.\n"
            "  Re-seed the stand and point the run at the manifest that seed wrote "
            "(--stand-manifest / $INSIGHT_STAND_MANIFEST)."
        ) from exc

    metafunc.parametrize("persona_name", names, ids=names)


def _credentials() -> SmokeCredentials:
    """Resolve the environment once per session, raising `PersonaError` on a gap."""
    global _CREDENTIALS
    if _CREDENTIALS is None:
        _CREDENTIALS = resolve_credentials()
    return _CREDENTIALS


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    """Skip this directory when the run was not aimed; validate the config when it was.

    Both halves act only on THIS directory's items — `items` is the whole
    session's collection, and a conftest that reached outside its own tree would
    be a trap.

    The configuration check lives here rather than in the `smoke_credentials`
    fixture because a `UsageError` raised in a fixture is not a session abort: it
    becomes one ERROR per test, so a single missing password would be reported
    ten identical times and the actual sentence would scroll past. Raised from a
    collection hook it stops the session once, before any request is made — the
    same contract the parent conftest gives `requires_seed`.
    """
    del config
    here = Path(__file__).parent
    mine = [item for item in items if here in Path(str(item.path)).parents]
    if not mine:
        return

    if not _aimed():
        reason = _not_aimed_reason()
        for item in mine:
            item.add_marker(pytest.mark.skip(reason=reason))
        return

    try:
        _credentials()
    except PersonaError as exc:
        raise pytest.UsageError(
            f"the deployed-stand smoke is aimed at {os.environ.get(BASE_URL_ENV)} but its "
            f"configuration is incomplete:\n{exc}"
        ) from exc


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def smoke_credentials() -> SmokeCredentials:
    """The stand's address and the way in, from the environment. Never defaulted.

    Already validated at collection time (above), so by the time a test asks for
    it this cannot fail — the `except` is here for a run that reaches the fixture
    by some other route, not as the primary report.
    """
    try:
        return _credentials()
    except PersonaError as exc:
        raise pytest.UsageError(f"smoke configuration is incomplete:\n{exc}") from exc


@pytest.fixture(scope="session")
def smoke_base_url(smoke_credentials: SmokeCredentials) -> str:
    """The deployed stand's public address — `$SMOKE_BASE_URL`, and only that.

    Deliberately NOT pytest-base-url's `base_url` fixture. That one is resolved
    by the parent conftest and, in a whole-suite run, points at the compose
    stand; addressing it from here would smoke-test the wrong deployment and
    pass.
    """
    return smoke_credentials.base_url


@pytest.fixture(scope="session")
def smoke_personas(stand_manifest: Manifest) -> Mapping[str, str]:
    """Role label -> manifest fixture name, for the personas under test."""
    return smoke_roster(stand_manifest)


@pytest.fixture(scope="session")
def smoke_login(
    stand_manifest: Manifest, smoke_credentials: SmokeCredentials
) -> Callable[[str], SmokeLogin]:
    """`smoke_login("dev_lead")` → that persona's login attempt, made once.

    Cached per session, so a persona authenticates at the IdP once no matter how
    many checks ask about them. The attempt is returned whether or not it
    succeeded — see `SmokeLogin`; the login check is what turns a failure into a
    readable assertion.
    """
    cache: dict[str, SmokeLogin] = {}

    def _login(name: str) -> SmokeLogin:
        if name not in cache:
            cache[name] = open_smoke_session(name, stand_manifest, smoke_credentials)
        return cache[name]

    return _login
