"""Every operation the stand serves behind the gateway, named once.

Read from the route tables in
`src/backend/services/{analytics,identity-resolution}/src/api/`, not from the
committed OpenAPI documents — the identity one is still the .NET contract and
is stale in both directions (it declares `/v1/persons/{email}`, which identity
answers 404 for and analytics actually serves; it omits both persons-sync
operations; and every operation in it lists only `200`).

Two consumers, and the reason this is one list rather than two:

* `test_gateway.py` asserts 401 for EVERY row. That is the deployed-path
  assertion the in-process rig cannot make, since auth is disabled there.
* the per-service modules assert what each operation does WITH a session.

A 401 alone proves nothing — the gateway rejects at the edge before routing, so
a path that does not exist answers 401 too. The refusal only means "refused"
when the same url is shown to serve something. Keeping the catalog here, and
having the service modules build their urls from it, is what stops the two
halves drifting onto different routes.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Final

from insight_stand import analytics_path, identity_path

# Concrete stand-ins for path parameters. The 401 sweep runs before any
# authentication, so the gateway never reaches a handler and these are never
# resolved — they only have to be well-formed enough to route.
SOME_ID: Final[str] = "01900000-0000-7000-8000-000000000000"
#: Still used by identity's `/internal/persons/by-email/{email}`, which is a
#: LOOKUP by email rather than a person-keyed route and so was untouched by the
#: identity cutover (#2098).
SOME_EMAIL: Final[str] = "nobody@example.com"

#: Stand-in -> the parameter it stands in for. These values are synthetic and
#: chosen for this purpose, so a path segment equal to one of them IS the
#: parameter — which is what lets the template be derived rather than declared
#: twice per row.
_PARAMETERS: Final[dict[str, str]] = {
    SOME_ID: "{id}",
    SOME_EMAIL: "{email}",
}


def _templated(path: str) -> str:
    """`/v1/queries/01900000-…` -> `/v1/queries/{id}`."""
    return "/".join(_PARAMETERS.get(segment, segment) for segment in path.split("/"))


@dataclass(frozen=True)
class Operation:
    """One (method, path) the gateway routes, with its url already built.

    Carries BOTH forms. `path` is the concrete url the sweep calls; `template`
    is what the operation is, and it is what the coverage gate groups by. A
    test updating a real saved query records a url containing that query's id,
    which matches the stand-in nowhere — so without the template the gate sees
    only the sweep's call against the catalogued url and reports an exercised
    operation as swept-only.
    """

    method: str
    path: str
    #: `analytics` | `identity` — which service answers, per routes.yaml.
    service: str

    @property
    def label(self) -> str:
        """`GET /api/analytics/v1/queries`, for a readable parametrize id."""
        return f"{self.method} {self.path}"

    @property
    def template(self) -> str:
        return _templated(self.path)


def _a(method: str, suffix: str) -> Operation:
    return Operation(method=method, path=analytics_path(suffix), service="analytics")


def _i(method: str, suffix: str) -> Operation:
    return Operation(method=method, path=identity_path(suffix), service="identity")


#: analytics — 10 operations.
ANALYTICS_OPERATIONS: Final[tuple[Operation, ...]] = (
    _a("GET", "/v1/queries"),
    _a("POST", "/v1/queries"),
    _a("GET", f"/v1/queries/{SOME_ID}"),
    _a("PUT", f"/v1/queries/{SOME_ID}"),
    _a("DELETE", f"/v1/queries/{SOME_ID}"),
    _a("POST", f"/v1/queries/{SOME_ID}/run"),
    _a("GET", "/v1/metric-definitions"),
    _a("POST", "/v1/metric-results"),
    _a("POST", "/v1/metric-drilldown"),
    # The only operation here that does not answer JSON — it serves CSV or
    # XLSX. It is catalogued all the same: the edge refuses an anonymous caller
    # before content negotiation happens, which is exactly what the sweep
    # asserts about every other one.
    _a("POST", "/v1/metric-drilldown/export"),
)

#: identity-resolution — 18 operations. `/health` and `/healthz` are the host
#: router's, not the product API, and are deliberately absent: the real probes
#: address the pod directly rather than passing the gateway.
IDENTITY_OPERATIONS: Final[tuple[Operation, ...]] = (
    _i("POST", "/v1/profiles"),
    _i("GET", "/v1/subchart"),
    _i("GET", f"/v1/subchart/{SOME_ID}"),
    _i("GET", "/v1/persons-seed"),
    _i("GET", f"/v1/persons-seed/{SOME_ID}"),
    _i("GET", "/v1/persons-sync"),
    _i("GET", f"/v1/persons-sync/{SOME_ID}"),
    _i("GET", "/v1/roles"),
    _i("POST", "/v1/roles"),
    _i("DELETE", f"/v1/roles/{SOME_ID}"),
    _i("GET", "/v1/person-roles"),
    _i("POST", "/v1/person-roles"),
    _i("DELETE", f"/v1/person-roles/{SOME_ID}"),
    _i("GET", "/v1/visibility"),
    _i("POST", "/v1/visibility"),
    _i("DELETE", f"/v1/visibility/{SOME_ID}"),
    # `.authenticated()`, not admin-gated — and the substring test below does not
    # catch it, which is correct: `/visible-persons` is not `/visibility`.
    _i("POST", "/v1/visible-persons"),
    _i("GET", f"/internal/persons/by-email/{SOME_EMAIL}"),
)

ALL_OPERATIONS: Final[tuple[Operation, ...]] = ANALYTICS_OPERATIONS + IDENTITY_OPERATIONS

#: The 13 identity operations behind `require_admin`, which resolves the caller
#: from the gateway JWT and requires an active `admin` row in `person_roles` —
#: it never reads the `insight-admin` REALM role. The seed grants nobody that
#: row, so every persona is refused; see out/endpoint-coverage-preconditions.md.
ADMIN_GATED: Final[frozenset[str]] = frozenset(
    op.label
    for op in IDENTITY_OPERATIONS
    if any(
        seg in op.path
        for seg in ("/persons-seed", "/persons-sync", "/roles", "/person-roles", "/visibility")
    )
)
