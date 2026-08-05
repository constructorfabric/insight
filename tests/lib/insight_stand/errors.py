"""Exception types for the stand test library.

Every one of these is a *loud* failure by design. The suite's contract is that
it never guesses: a stand it cannot describe, cannot reach, or cannot log in to
must stop the run with an actionable message rather than fall through to a
default that silently tests the wrong thing.
"""

from __future__ import annotations


class StandError(Exception):
    """Base class for every error this library raises."""


class ManifestError(StandError):
    """The seed manifest is missing, unparseable, or does not match the schema.

    Raised — never swallowed — so `tests/stand` refuses to start against a
    stand that cannot describe itself. Falling back to hardcoded fixture names
    or capabilities would produce a green run that proves nothing.
    """


class StandConnectionError(StandError):
    """The stand's base URL could not be resolved from the environment.

    Distinct from `ManifestError`: the manifest describes what was *seeded*,
    while the base URL is a property of *where the stand is published* — a host
    port for a host-side runner, a compose-network address for an in-network
    one. See `insight_stand.stand.resolve_base_url` for the resolution order.
    """


class PersonaError(StandError):
    """A persona cannot be used as a test identity.

    Covers both halves of "who is this": no credential could be sourced for
    them, or the roles the stand grants them are not the roles the roster says
    they should have. Both are hard failures — a fixture that logs in as
    somebody with unexpected authority makes every visibility assertion built
    on it meaningless.
    """


class LoginNotCompletedError(StandError):
    """A real login was started but could not be carried to a session.

    Carries the URL the flow stopped at, so the caller can see exactly which
    step is unfinished. Phase 6 owns completing the Keycloak challenge; until
    then this is the honest outcome of `LoginSession.login()` against a stand
    whose IdP presents a login form.
    """

    def __init__(self, message: str, *, stopped_at: str | None = None) -> None:
        super().__init__(message)
        self.stopped_at = stopped_at
