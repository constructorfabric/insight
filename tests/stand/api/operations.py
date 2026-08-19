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

#: Stand-in for a source-native `{account_id}` path segment — an arbitrary
#: string, not a UUID, so it needs its own recognisable value.
SOME_ACCOUNT_ID: Final[str] = "stand-in-account"

#: Stand-in for `{metric_key}`, which is a dotted `family.name` string rather
#: than a UUID. Kept a DOTTED key on purpose: the literal `export`/`import`
#: segments of the sibling routes must not collide with it, so the template
#: derives correctly and a literal path wins over this one.
SOME_METRIC_KEY: Final[str] = "scratch.probe"

#: Stand-in -> the parameter it stands in for. These values are synthetic and
#: chosen for this purpose, so a path segment equal to one of them IS the
#: parameter — which is what lets the template be derived rather than declared
#: twice per row.
_PARAMETERS: Final[dict[str, str]] = {
    SOME_ID: "{id}",
    SOME_METRIC_KEY: "{metric_key}",
    SOME_ACCOUNT_ID: "{account_id}",
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


#: analytics — 20 operations.
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
    # Custom-metric CRUD + export/import. `export`/`import` stay literal paths;
    # `{metric_key}` is a dotted string, so its stand-in is a dotted key and a
    # literal sibling always wins the template match.
    _a("GET", "/v1/metrics"),
    _a("POST", "/v1/metrics"),
    _a("GET", "/v1/metrics/export"),
    _a("POST", "/v1/metrics/import"),
    _a("GET", f"/v1/metrics/{SOME_METRIC_KEY}"),
    _a("PUT", f"/v1/metrics/{SOME_METRIC_KEY}"),
    _a("DELETE", f"/v1/metrics/{SOME_METRIC_KEY}"),
    # Usage monitoring. All three are `.authenticated()` at the edge; the
    # summary's admin gate lives inside the handler, so it is invisible here and
    # asserted in test_usage.py instead.
    _a("POST", "/v1/usage/events"),
    _a("GET", "/v1/usage/config"),
    _a("GET", "/v1/usage/summary"),
)

#: identity-resolution — 26 operations. `/health` and `/healthz` are the host
#: router's, not the product API, and are deliberately absent: the real probes
#: address the pod directly rather than passing the gateway.
IDENTITY_OPERATIONS: Final[tuple[Operation, ...]] = (
    _i("POST", "/v1/profiles"),
    _i("GET", "/v1/me"),
    _i("GET", "/v1/persons"),
    # The operator correction surface. `source` stays the literal `github` in
    # the accounts read: it is a connector type, not an id, and the tests
    # address the same literal — a stand-in would fold a segment nothing varies.
    _i("GET", "/v1/resolution/attention"),
    _i("GET", "/v1/resolution/accounts"),
    _i("GET", f"/v1/resolution/accounts/github/{SOME_ID}/{SOME_ACCOUNT_ID}"),
    _i("GET", f"/v1/resolution/persons/{SOME_ID}/accounts"),
    _i("POST", "/v1/resolution/bind"),
    _i("POST", "/v1/resolution/merge"),
    _i("POST", "/v1/resolution/detach"),
    _i("POST", "/v1/resolution/exclude"),
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
    # `.authenticated()`, not admin-gated — deliberately absent from
    # `_ADMIN_GATED_SUFFIXES` below. The GET enumerates the same visible set the
    # POST filters against, under the same rule.
    _i("GET", "/v1/visible-persons"),
    _i("POST", "/v1/visible-persons"),
)

ALL_OPERATIONS: Final[tuple[Operation, ...]] = ANALYTICS_OPERATIONS + IDENTITY_OPERATIONS

#: Suffixes of the identity operations behind `require_admin`, enumerated
#: exactly — a substring rule would silently classify future routes (and
#: `/visible-persons` is one hyphen away from a false match today).
_ADMIN_GATED_SUFFIXES: Final[tuple[str, ...]] = (
    "/v1/persons",
    "/v1/resolution/attention",
    "/v1/resolution/accounts",
    f"/v1/resolution/accounts/github/{SOME_ID}/{SOME_ACCOUNT_ID}",
    f"/v1/resolution/persons/{SOME_ID}/accounts",
    "/v1/resolution/bind",
    "/v1/resolution/merge",
    "/v1/resolution/detach",
    "/v1/resolution/exclude",
    "/v1/persons-seed",
    f"/v1/persons-seed/{SOME_ID}",
    "/v1/persons-sync",
    f"/v1/persons-sync/{SOME_ID}",
    "/v1/roles",
    f"/v1/roles/{SOME_ID}",
    "/v1/person-roles",
    f"/v1/person-roles/{SOME_ID}",
    "/v1/visibility",
    f"/v1/visibility/{SOME_ID}",
)

#: The 21 identity operations behind `require_admin`, which resolves the caller
#: from the gateway JWT and requires an active `admin` row in `person_roles` —
#: it never reads the `insight-admin` REALM role. The seed grants that row to
#: exactly one persona, the admin operator; every other persona is refused.
#: See out/endpoint-coverage-preconditions.md.
ADMIN_GATED: Final[frozenset[str]] = frozenset(
    op.label
    for op in IDENTITY_OPERATIONS
    if op.path in {identity_path(suffix) for suffix in _ADMIN_GATED_SUFFIXES}
)
