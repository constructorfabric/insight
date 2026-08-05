"""HTTP client for the stand's API surface — through the gateway, only.

The gateway is the edge that terminates the session and the sole thing the
product actually exposes. Analytics and identity do publish their own host
ports on compose, but addressing them directly would skip authentication
entirely and prove nothing about the deployed path, so this client refuses to
build such a request: every path must sit under a prefix the gateway routes.

Prefixes come from `deploy/compose/gateway/routes.yaml`:

    /api/analytics  ->  http://analytics:8081   (strip_prefix: true)
    /api/identity   ->  http://identity-resolution:8082  (strip_prefix: true)

`strip_prefix` means the service sees its own `/v1/...`, so a caller writes the
full gateway path (`/api/analytics/v1/metrics`) and the rewrite is the
gateway's business.

Response bodies are read two ways. `ApiResponse.parse(Model)` validates against
a pydantic model and is how a test that cares about a payload should read it;
`ApiResponse.json()` returns the raw `JsonValue` and stays for the bodies no
model describes — error envelopes the service emits below its canonical layer,
and the media-type cases whose whole point is that the body was refused.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING, Protocol

from . import coverage

if TYPE_CHECKING:  # pragma: no cover - typing only
    from pydantic import BaseModel


class StandSession(Protocol):
    """Anything that can authenticate a request.

    Two implementations, and the interface exists only because there are two:
    `LoginSession` (a person, via the real IdP login) and `ServiceTokenSession`
    (a service principal, exchanged at the authenticator's token endpoint).
    Every product surface can be called by both, so the client must not care
    which it holds.

    Anonymity is deliberately NOT a third implementation. `session is None`
    means no header is attached at all, which is what makes the 401 sweep in
    `stand/api/test_gateway.py` mean something — an `AnonymousSession` returning
    `{}` could one day return a stale token instead, and `None` cannot.
    """

    def headers(self) -> Mapping[str, str]: ...


#: Anything `json.loads` can return. Recursive, so a caller indexing into a
#: decoded body keeps a real type instead of falling off into `Any`.
type JsonValue = str | int | float | bool | None | list[JsonValue] | dict[str, JsonValue]

#: What httpx accepts in a query string: scalars, or a repeated key.
type QueryValue = str | int | float | bool | None
type QueryParams = Mapping[str, QueryValue | Sequence[QueryValue]]

ANALYTICS_PREFIX = "/api/analytics"
IDENTITY_PREFIX = "/api/identity"
GATEWAY_API_PREFIXES: tuple[str, ...] = (ANALYTICS_PREFIX, IDENTITY_PREFIX)

# Served by the edge itself rather than routed to a backend: the SPA at `/` and
# the authenticator's browser endpoints under `/auth/`.
EDGE_PATH_PREFIXES: tuple[str, ...] = ("/auth/",)


def analytics_path(suffix: str) -> str:
    """`analytics_path("/v1/metrics") -> "/api/analytics/v1/metrics"`."""
    return f"{ANALYTICS_PREFIX}/{suffix.lstrip('/')}"


def identity_path(suffix: str) -> str:
    """`identity_path("/v1/persons") -> "/api/identity/v1/persons"`."""
    return f"{IDENTITY_PREFIX}/{suffix.lstrip('/')}"


@dataclass(frozen=True)
class ApiResponse:
    """A transport-agnostic view of one response.

    Tests assert against this rather than an `httpx.Response`, so swapping the
    HTTP library never touches a test.
    """

    status_code: int
    headers: Mapping[str, str]
    content: bytes
    text: str
    url: str

    def json(self) -> JsonValue:
        """Decoded JSON body, or `None` when the body is not JSON.

        Returning `None` rather than raising keeps a status-code assertion
        readable when the stand answers with an HTML error page.
        """
        import json

        try:
            decoded: JsonValue = json.loads(self.text)
        except ValueError:
            return None
        return decoded

    def parse[T: BaseModel](self, model: type[T]) -> T:
        """The body validated against `model`, or a readable test failure.

        Replaces the `isinstance(body, dict)` / `body.get("items")` /
        `str(item["name"])` ladder that every test otherwise grows its own
        version of. The model states the shape once and the caller gets typed
        attributes.

        Failure is an `AssertionError` rather than pydantic's `ValidationError`
        on purpose. A raw ValidationError in a test report says which field was
        wrong but not which request produced it, which is strictly less than the
        hand-written guards it replaces; this carries the url, every field-level
        error, and the body that caused them.
        """
        from pydantic import ValidationError

        try:
            return model.model_validate_json(self.text)
        except ValidationError as exc:
            problems = "\n".join(
                f"  {'.'.join(str(part) for part in error['loc']) or '<root>'}: {error['msg']}"
                for error in exc.errors()
            )
            raise AssertionError(
                f"{self.url} did not answer with a valid {model.__name__} "
                f"(HTTP {self.status_code}, content-type {self.content_type or '<none>'}):\n"
                f"{problems}\n"
                f"  body: {self.text[:300]}"
            ) from None

    @property
    def content_type(self) -> str:
        return self.headers.get("content-type", "")


@dataclass
class ApiClient:
    """Issues requests at one stand, optionally carrying a session.

    `session is None` means genuinely unauthenticated — no header is attached
    at all — so a 401 assertion is testing the stand rather than a mistake in
    the test. The session is re-read on every request, so a session that
    re-acquires mid-run is picked up without rebuilding the client.
    """

    base_url: str
    session: StandSession | None = None
    timeout_s: float = 30.0

    #: False only for a client addressing a service's own listener rather than
    #: the edge. The gateway is a browser BFF and refuses a bearer-carrying
    #: caller, so a SERVICE principal has no edge address to use — see
    #: `service_token.default_identity_url`. Off by default: reaching a backend
    #: port is the mistake `_checked_path` exists to catch, and a test that
    #: wants to must say so.
    edge_fronted: bool = True

    def __post_init__(self) -> None:
        self.base_url = self.base_url.rstrip("/")

    # -- construction ------------------------------------------------------

    def with_session(self, session: StandSession) -> ApiClient:
        """A sibling client at the same stand, authenticated as that session."""
        return ApiClient(
            base_url=self.base_url,
            session=session,
            timeout_s=self.timeout_s,
            edge_fronted=self.edge_fronted,
        )

    # -- requests ----------------------------------------------------------

    def request(
        self,
        method: str,
        path: str,
        *,
        params: QueryParams | None = None,
        json_body: JsonValue = None,
        content: str | None = None,
        headers: Mapping[str, str] | None = None,
    ) -> ApiResponse:
        """Issue one request. `json_body` and `content` are exclusive.

        `content` sends the body verbatim, with whatever `Content-Type` the
        caller sets — which is the only way to assert the 415 contract, since a
        body the service must REFUSE on its media type cannot be expressed as
        `json=`.
        """
        if json_body is not None and content is not None:
            raise ValueError("pass json_body or content, not both")

        import httpx

        checked_path = self._checked_path(path)
        url = f"{self.base_url}{checked_path}"
        merged: dict[str, str] = dict(self.session.headers()) if self.session else {}
        if headers:
            merged.update(headers)

        with httpx.Client(timeout=self.timeout_s, follow_redirects=False) as client:
            response = client.request(
                method,
                url,
                params=params,
                json=json_body,
                content=content,
                headers=merged,
            )
        # Every request the suite makes passes through here, which is what lets
        # the coverage ledger be a byproduct of the tests rather than a list
        # somebody maintains. Metadata only — never the body.
        coverage.record(method, checked_path, response.status_code)

        return ApiResponse(
            status_code=response.status_code,
            headers={k.lower(): v for k, v in response.headers.items()},
            content=response.content,
            text=response.text,
            url=str(response.url),
        )

    # The verbs spell their arguments out rather than forwarding `**kwargs`:
    # `**kwargs: Any` would type-check `client.get(path, jsonbody=…)` and any
    # other typo, and gives an editor nothing to complete. GET and DELETE take
    # no body on purpose — a request that should not have one should not be
    # able to express one.

    def get(
        self,
        path: str,
        *,
        params: QueryParams | None = None,
        headers: Mapping[str, str] | None = None,
    ) -> ApiResponse:
        return self.request("GET", path, params=params, headers=headers)

    def delete(
        self,
        path: str,
        *,
        params: QueryParams | None = None,
        headers: Mapping[str, str] | None = None,
    ) -> ApiResponse:
        return self.request("DELETE", path, params=params, headers=headers)

    def post(
        self,
        path: str,
        *,
        json_body: JsonValue = None,
        content: str | None = None,
        params: QueryParams | None = None,
        headers: Mapping[str, str] | None = None,
    ) -> ApiResponse:
        return self.request(
            "POST", path, json_body=json_body, content=content, params=params, headers=headers
        )

    def put(
        self,
        path: str,
        *,
        json_body: JsonValue = None,
        content: str | None = None,
        params: QueryParams | None = None,
        headers: Mapping[str, str] | None = None,
    ) -> ApiResponse:
        return self.request(
            "PUT", path, json_body=json_body, content=content, params=params, headers=headers
        )

    # -- guards ------------------------------------------------------------

    def _checked_path(self, path: str) -> str:
        """Reject anything the gateway does not front.

        A bare service path such as `/v1/metrics` is the classic way to end up
        addressing a backend port directly; catching it here turns that into an
        immediate, explanatory error instead of a puzzling 404.

        Skipped for a client that declared itself off the edge, which addresses
        a service directly and therefore uses the service's own paths.
        """
        if not path.startswith("/"):
            raise ValueError(f"path must be absolute, got {path!r}")
        if not self.edge_fronted:
            return path
        if path == "/" or path.startswith(EDGE_PATH_PREFIXES):
            return path
        if path.startswith(GATEWAY_API_PREFIXES):
            return path
        allowed = ", ".join((*GATEWAY_API_PREFIXES, *EDGE_PATH_PREFIXES, "/"))
        raise ValueError(
            f"{path!r} is not a gateway-fronted path. Requests must go through "
            f"the gateway, never straight at a backend port; use one of: {allowed} "
            "(see deploy/compose/gateway/routes.yaml)."
        )


__all__: Sequence[str] = (
    "ANALYTICS_PREFIX",
    "EDGE_PATH_PREFIXES",
    "GATEWAY_API_PREFIXES",
    "IDENTITY_PREFIX",
    "ApiClient",
    "ApiResponse",
    "StandSession",
    "analytics_path",
    "identity_path",
)
