"""The edge: every operation is refused without a session, actionably.

The rig sweeps 401 over its own 29 analytics operations, and its host verifies
the gateway JWT to answer them. What it still cannot make is the 403 half of
this assertion: with no role gate in front of an in-process service, a refusal
by identity has no way to happen, so 403 sits in its BLOCKED list per route.
Here both are the point — the runtime proof that a deployed stand requires a
real login, and that `authDisabled: true` was never switched on.

Swept over the whole catalogue rather than written per service, because the
property is the gateway's and it is uniform: it terminates the session at the
edge and refuses before it routes anything. One operation added to
`operations.py` is one more url proven closed, with nothing else to remember.

The refusal only MEANS something because the same urls are shown to serve real
data elsewhere in this directory — the gateway answers 401 for paths that do
not exist too.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient

from .operations import ALL_OPERATIONS, Operation
from .schemas import PROBLEM_CONTENT_TYPE, ProblemDocument

#: Bodies are irrelevant: the edge rejects before any handler reads one, and
#: sending one would only obscure which layer answered.
_METHODS_WITHOUT_BODY = frozenset({"GET", "DELETE"})


@pytest.mark.parametrize("operation", ALL_OPERATIONS, ids=lambda op: op.label)
def test_operation_is_refused_without_a_session(
    api_client: ApiClient, operation: Operation
) -> None:
    """401, with a problem document a client can act on.

    Both halves on every operation, not just the status. A refusal whose body a
    client cannot read is only half a rejection: the SPA decides between
    "redirect to sign-in" and "show an error" from `status` and `title`, so a
    route that answered 401 with an HTML page or an empty body would pass a
    status-only assertion and still break the product.

    Validating rather than spot-checking two keys is what makes that cheap
    enough to do 45 times — `ProblemDocument` also forbids undeclared fields, so
    an envelope that quietly grows or loses one fails here.
    """
    response = api_client.request(
        operation.method,
        operation.path,
        json_body=None if operation.method in _METHODS_WITHOUT_BODY else {},
    )
    assert response.status_code == 401, (
        f"{operation.label} answered {response.status_code} to an anonymous caller, "
        f"expected 401: {response.text[:300]}"
    )
    assert response.content_type == PROBLEM_CONTENT_TYPE, (
        f"{operation.label} rejected with content-type {response.content_type!r}, "
        f"expected {PROBLEM_CONTENT_TYPE!r}: {response.text[:300]}"
    )

    problem = response.parse(ProblemDocument)
    assert problem.status == 401, (
        f"{operation.label}: problem document says status {problem.status} but the "
        "response was 401 — a client trusting the body would misroute"
    )
    assert problem.title, f"{operation.label}: problem document has no title"


def test_the_refusal_names_the_way_back_in(api_client: ApiClient) -> None:
    """The 401 tells a client what to do about it, not just that it failed.

    Asserted once rather than per operation: the gateway writes the same document
    for everything it fronts, so 45 copies would restate one fact. What the sweep
    above proves is that every route reaches THIS behaviour.
    """
    response = api_client.get(ALL_OPERATIONS[0].path)
    assert response.status_code == 401

    problem = response.parse(ProblemDocument)
    assert "/auth/login" in problem.detail, (
        "the 401 does not name the authentication entry point, so a client is left "
        f"guessing where to send the user: {problem.detail!r}"
    )
    assert problem.context.get("reason") == "no_session", (
        "the 401 does not distinguish 'no session' from any other authentication "
        f"failure, which is what lets a client tell 'sign in' from 'try again': {problem.context}"
    )
