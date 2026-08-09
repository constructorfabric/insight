"""The post-deploy gate: the edge answers, people log in, and the data is there.

    GET  /auth/login                      302 to the IdP's authorize endpoint
    GET  /auth/login  (per persona)       the whole OIDC chain, to a session cookie
    GET  /auth/me                         200, naming the authenticated persona
    POST /api/analytics/v1/metric-results 200, with a value over the seeded window

Four checks, one per contract, in DEFINITION ORDER — pytest runs a module's
tests in the order they appear, and that order is the point. Each check narrows
the previous one's answer, so the FIRST failure is the diagnosis:

1. the edge is up and the authenticator is wired to an IdP;
2. a credential this stand accepts exists, and the OIDC chain completes;
3. the session that came back belongs to the person who logged in;
4. the seeded data reaches the API as a number, for the window the seed wrote.

A stand where check 1 passes and check 2 fails has an IdP problem. One where
2 passes and 3 fails has an identity-resolution problem. One where 3 passes and
4 fails deployed and authenticated fine but has no data — three genuinely
different pages, and this ordering is what tells them apart without reading a
log.

What this module deliberately does NOT do:

* **Assert a metric's value.** The seed's golden set is empty by design, and
  reading a number off a running stand to assert it back proves only that the
  code which produced it produced it. Check 4 asserts SHAPE and NON-NULLNESS
  over the manifest's own `data_window`, never a number.
* **Validate bodies against the generated OpenAPI models.** That is `api/`'s
  job and it is a contract test. A deploy gate that went red because a
  generated model gained a field would be crying wolf about the one thing it is
  supposed to be trusted on, so the shape checks here are hand-written, narrow,
  and about the fields a human would notice missing.
* **Write anything.** Every request is a read. The gate runs against a stand CI
  is about to hand to people.
* **Skip when it cannot log in.** See `login.py` — an unusable realm is a
  configuration failure with a named cause, not an absent capability.
"""

from __future__ import annotations

import math
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import Final
from urllib.parse import parse_qs, urlsplit

import httpx
import pytest
from insight_stand import ApiClient, ApiResponse, Manifest, analytics_path
from insight_stand.api import JsonValue
from insight_stand.session import CALLBACK_PATH, LOGIN_PATH, SESSION_COOKIE_NAME

from .conftest import METRIC_PROBE_ROLE
from .login import BASE_URL_ENV, LoginMode, SmokeCredentials, SmokeLogin

pytestmark = pytest.mark.stand_smoke

ME_PATH: Final[str] = "/auth/me"
METRIC_RESULTS: Final[str] = analytics_path("/v1/metric-results")

#: Query parameters an OpenID Connect authorization request carries. Asserted by
#: NAME and never by value, because every value in here is stand-specific: the
#: client id, the redirect host and the PKCE challenge all differ per
#: deployment, and pinning any of them would make this check a configuration
#: snapshot rather than a statement about the protocol.
_AUTHORIZE_PARAMS: Final[tuple[str, ...]] = (
    "response_type",
    "client_id",
    "redirect_uri",
    "state",
    "scope",
    "code_challenge",
    "code_challenge_method",
)

#: Metric keys the seeder's generators write data behind, one per generator
#: family (`generators/git.py`, `generators/task.py`, `generators/collab.py`).
#:
#: Three rather than one so a single generator regressing cannot make this check
#: vacuous, and three rather than the whole catalogue so the gate stays a few
#: seconds long. Each is probed in its OWN request: an unknown `metric_key`
#: makes the whole batch 400, so batching them would let one retired key hide
#: the answer from the other two.
#:
#: Only ONE has to answer. A metric being retired from the registry is a product
#: decision that should not turn a deploy gate red — the failure message names
#: every key and what it did, so a shrinking list is visible rather than silent.
SEED_GUARANTEED_METRIC_KEYS: Final[tuple[str, ...]] = (
    "git.commits",
    "tasks.closed",
    "collab.messages_sent",
)


# ---------------------------------------------------------------------------
# 1. The edge answers and the authenticator is wired to an IdP
# ---------------------------------------------------------------------------


def test_the_login_route_redirects_to_an_oidc_authorize_endpoint(smoke_base_url: str) -> None:
    """`GET /auth/login`, unauthenticated, starts a real authorization-code flow.

    The cheapest possible statement that the deployment is alive end to end: the
    public URL resolves, the edge routes `/auth/*` to the authenticator, the
    authenticator has an issuer configured for this host, and it can reach its
    login-state store — a 429 or a 500 here all mean something different from a
    302.

    Asserted by SHAPE, never by host. The stand's IdP hostname is deployment
    configuration and belongs in the environment, not in a test in a public
    repository; what the product actually promises is an absolute redirect to an
    OIDC authorize endpoint carrying PKCE, and that is what is checked.
    """
    try:
        response = ApiClient(base_url=smoke_base_url).get(LOGIN_PATH)
    except httpx.HTTPError as exc:
        # The first request of the run, so this is where "the deploy never
        # became reachable" lands. Turned into a stated failure rather than left
        # as a transport traceback: the useful facts are the address and where
        # it came from, and neither of those is in an httpx stack trace.
        pytest.fail(
            f"the stand did not answer at all: {type(exc).__name__}: {exc}\n"
            f"  address: {smoke_base_url} (from ${BASE_URL_ENV})\n"
            f"  Nothing below this can run. Either the deployment never became "
            f"reachable at that address, or ${BASE_URL_ENV} names the wrong one."
        )

    assert response.status_code in (301, 302, 303, 307, 308), (
        f"GET {LOGIN_PATH} answered {response.status_code} instead of redirecting to the IdP. "
        f"A 404 means the edge does not route /auth/*; a 5xx means the authenticator is up "
        f"but could not start a login; a 200 means something other than the authenticator "
        f"answered. Body: {response.text[:300]}"
    )

    location = response.headers.get("location", "")
    assert location, (
        f"GET {LOGIN_PATH} answered {response.status_code} with no Location header — "
        f"there is nothing for a browser to follow. Headers: {sorted(response.headers)}"
    )

    target = urlsplit(location)
    assert target.scheme in ("http", "https") and target.netloc, (
        f"the login redirect target is not an absolute URL: {location!r}. The authenticator "
        f"builds it from the issuer it discovered, so a relative or empty target means the "
        f"issuer is misconfigured for this host."
    )

    query = parse_qs(target.query)
    missing = [name for name in _AUTHORIZE_PARAMS if not query.get(name, [""])[0]]
    assert not missing, (
        f"the login redirect is missing {missing} — that is not an OIDC authorization "
        f"request. Redirected to {target.scheme}://{target.netloc}{target.path} with "
        f"parameters {sorted(query)}."
    )
    assert query["response_type"][0] == "code", (
        f"the authenticator asked for response_type={query['response_type'][0]!r}; this "
        f"product only implements authorization code + PKCE."
    )
    assert query["code_challenge_method"][0] == "S256", (
        f"PKCE challenge method is {query['code_challenge_method'][0]!r}, not S256 — "
        f"the flow started without a usable proof key."
    )
    assert "openid" in query["scope"][0].split(), (
        f"the authorization request asked for scope {query['scope'][0]!r}, which does not "
        f"include 'openid', so no id_token would come back and no person could be resolved."
    )

    callback = urlsplit(query["redirect_uri"][0])
    assert callback.path == CALLBACK_PATH, (
        f"the IdP is told to send the code to {callback.path!r} rather than {CALLBACK_PATH!r}; "
        f"the authenticator's configured redirect_uri does not match the route it serves, so "
        f"every login would dead-end after the IdP."
    )


# ---------------------------------------------------------------------------
# 2. A seeded persona can actually log in
# ---------------------------------------------------------------------------


def test_each_seeded_persona_can_log_in(
    smoke_login: Callable[[str], SmokeLogin], persona_name: str
) -> None:
    """The whole OIDC chain, once per persona, through the public URL.

    Nothing is stubbed and nothing is minted: `/auth/login` → the IdP's real
    login page → the form submit → `/auth/callback` → `__Host-sid`. Several
    personas rather than one because a single login only proves that ONE
    credential works — a realm that granted roles to one user, an identity
    resolution that resolved one person, and a tenant claim that happened to
    match are all indistinguishable from a working stand until a second person
    tries.

    The whole diagnosis lives in the assertion message, including which stand
    configuration is missing when the IdP cannot serve a password form at all.
    `require()` rather than `assert attempt.ok, attempt.failure`: the bare assert
    makes pytest append its own rewritten `where False = SmokeLogin(...)` line,
    which repeats the diagnosis inside a truncated dataclass repr and buries it.
    """
    persona = smoke_login(persona_name).require()

    assert persona.session.is_authenticated(), (
        f"persona {persona_name!r} completed the login chain but holds no live "
        f"{SESSION_COOKIE_NAME} session — the callback answered without setting the cookie, "
        f"or it expired between being set and being read."
    )


# ---------------------------------------------------------------------------
# 3. The session belongs to the person who logged in
# ---------------------------------------------------------------------------


def test_auth_me_names_the_authenticated_persona(
    smoke_login: Callable[[str], SmokeLogin],
    persona_name: str,
    stand_manifest: Manifest,
    smoke_credentials: SmokeCredentials,
) -> None:
    """`/auth/me` reports the persona's own identity, not just *an* identity.

    This is the check that makes check 2 mean something. A session cookie proves
    the IdP accepted a credential; it does not prove the authenticator resolved
    the right person, put the right tenant on the session, or that identity
    holds a row for them at all. A stand where every login silently resolves to
    the same person, or to the wrong tenant, passes check 2 and fails here.

    The person id is asserted rather than only the email because it is the key
    every person-scoped route takes — the same value check 4 asks the metric
    about.
    """
    persona = smoke_login(persona_name).require()
    response = persona.client.get(ME_PATH)

    assert response.status_code == 200, (
        f"GET {ME_PATH} answered {response.status_code} for {persona.email} while carrying "
        f"a session cookie. A 401 means the authenticator no longer recognises the session "
        f"it just issued (a session store that lost it, or a TTL shorter than this run); "
        f"a 5xx means it could not read it. Body: {response.text[:300]}"
    )

    body = _json_object(response, f"{ME_PATH} for {persona.email}")

    reported_email = str(body.get("email", ""))
    assert reported_email.casefold() == persona.email.casefold(), (
        f"logged in as {persona.email!r} and {ME_PATH} reports {reported_email!r}. The "
        f"session was minted for a different person than the one who authenticated — "
        f"compare against the manifest at {stand_manifest.source_path}."
    )
    assert str(body.get("user", "")) == persona.person.uuid, (
        f"{ME_PATH} resolved {persona.email} to person id {body.get('user')!r}, but the "
        f"manifest says {persona.person.uuid!r}. Identity resolved the login to the wrong "
        f"row, so every person-scoped query in this session would be about somebody else."
    )
    assert str(body.get("tenant_id", "")) == stand_manifest.tenant, (
        f"{ME_PATH} put tenant {body.get('tenant_id')!r} on {persona.email}'s session, but "
        f"the stand was seeded for {stand_manifest.tenant!r}. Every tenant-scoped query "
        f"would come back empty and look like missing data."
    )

    if smoke_credentials.mode is LoginMode.OVERRIDE:
        # In view-as mode the session is deliberately an impersonation, and the
        # authenticator says so. Asserting it keeps the two modes honestly
        # different: a run that BELIEVES it is impersonating but is really just
        # logged in as the bootstrap principal would otherwise pass every check
        # above with the wrong person's data.
        assert str(body.get("impersonator_email", "")).casefold() == (
            smoke_credentials.bootstrap_email.casefold()
        ), (
            f"{LoginMode.OVERRIDE.value!r} mode expects {ME_PATH} to name the real principal "
            f"behind the view-as session as {smoke_credentials.bootstrap_email!r}, and it "
            f"reports {body.get('impersonator_email')!r}. Either override_enabled is off on "
            f"this stand (the authenticator then ignores __override and logs the attempt), "
            f"or the session is not the one this run thinks it is."
        )


# ---------------------------------------------------------------------------
# 4. The seeded data reaches the API
# ---------------------------------------------------------------------------


def test_a_seeded_metric_answers_over_the_seeded_window(
    smoke_login: Callable[[str], SmokeLogin],
    smoke_personas: Mapping[str, str],
    stand_manifest: Manifest,
) -> None:
    """A real number comes back for a real person over the window the seed wrote.

    The end of the chain this gate exists for. Everything before it can pass on
    a stand whose ClickHouse is empty, whose gold models were never rebuilt, or
    whose tenant on the session does not match the tenant the rows carry — and a
    user would see a dashboard of dashes. This is the check that notices.

    The period is the manifest's own `data_window`, so the request asks for the
    range the stand was actually seeded over rather than a guess, and the anchor
    moves with the seed instead of being pinned to a date in a test.

    Values are asserted NON-NULL and FINITE and nothing more. Not the number:
    the seed's golden set is empty by design (see
    `src/ingestion/tools/seed/golden_metrics.py`) and asserting a value read
    back off a running stand proves only that the code which produced it
    produced it. Not even non-negativity, tempting as it is for three counters:
    that is a fact about the metric definitions, it belongs to the dbt gold
    tests that already assert it (`assert_ic_kpis_bounds`,
    `assert_collab_messaging_bounds`), and encoding it here would make a
    definition change look like a broken deployment.
    """
    persona = smoke_login(smoke_personas[METRIC_PROBE_ROLE]).require()
    start, _, end = stand_manifest.data_window.partition("..")
    assert start and end, (
        f"the manifest at {stand_manifest.source_path} carries data_window "
        f"{stand_manifest.data_window!r}, which is not a `from..to` range — there is no "
        f"period to ask about."
    )

    probes = tuple(
        _probe(persona.client, persona.person.uuid, start, end, metric_key)
        for metric_key in SEED_GUARANTEED_METRIC_KEYS
    )
    report = "\n".join(f"  {probe.summary()}" for probe in probes)

    # Bound to a name rather than left as `any(... for ...)`: pytest's assertion
    # rewriting appends what it evaluated, and a bare generator prints as
    # `where False = any(<generator object ...>)`, which is noise directly under
    # the sentence that matters.
    answered = [probe for probe in probes if probe.plausible]
    assert answered, (
        f"no seed-guaranteed metric answered with data for {persona.email} over the seeded "
        f"window {stand_manifest.data_window}:\n{report}\n"
        f"  Every probe reached the API through the public URL with a real session, so this "
        f"is not an auth failure. The usual causes, in order: the stand was deployed but "
        f"never seeded; the seed wrote silver but the gold models were not rebuilt after it; "
        f"the rows carry a different tenant than the session does "
        f"({stand_manifest.tenant}); or all three metric keys have been retired from the "
        f"registry, which the statuses above would show as 400."
    )

    malformed = [probe for probe in probes if probe.status == 200 and probe.note]
    assert not malformed, (
        "a metric answered 200 with a body this gate could not read:\n"
        + "\n".join(f"  {probe.summary()}" for probe in malformed)
        + "\n  The shape checked here is only what a dashboard needs — a period value and a "
        "timeseries with points — so a failure means the response is genuinely not that."
    )


# ---------------------------------------------------------------------------
# Reading the answers
# ---------------------------------------------------------------------------


def _json_object(response: ApiResponse, what: str) -> Mapping[str, JsonValue]:
    """The body as a JSON object, or a failure naming what was asked for."""
    body = response.json()
    if not isinstance(body, dict):
        raise AssertionError(
            f"{what} did not answer with a JSON object (content-type "
            f"{response.content_type or '<none>'}): {response.text[:300]}"
        )
    return body


def _number(value: JsonValue) -> float | None:
    """A finite number, or None for anything else — including a JSON `true`."""
    if isinstance(value, bool) or not isinstance(value, int | float):
        return None
    return float(value) if math.isfinite(value) else None


def _objects(value: JsonValue) -> list[dict[str, JsonValue]]:
    """The JSON objects in a list, or nothing.

    Walking the body through this rather than indexing it keeps `_probe` free of
    `isinstance` ladders and makes "the field was there but held the wrong kind
    of thing" behave the same as "the field was absent" — both are reported as a
    missing part of the shape, which is what a reader of the failure needs.
    """
    if not isinstance(value, list):
        return []
    return [item for item in value if isinstance(item, dict)]


@dataclass(frozen=True)
class MetricProbe:
    """What one metric key did when asked about one person over one period."""

    metric_key: str
    status: int
    #: Empty when the body was the shape a dashboard needs; otherwise the first
    #: thing about it that was not.
    note: str = ""
    period_values: tuple[float | None, ...] = ()
    series: int = 0
    points: int = 0
    non_null_points: int = 0

    @property
    def plausible(self) -> bool:
        """Answered 200, in shape, with at least one real number on both views."""
        return (
            self.status == 200
            and not self.note
            and any(value is not None for value in self.period_values)
            and self.series >= 1
            and self.points >= 1
            and self.non_null_points >= 1
        )

    def summary(self) -> str:
        detail = self.note or (
            f"period values {list(self.period_values)}, {self.series} series, "
            f"{self.points} points ({self.non_null_points} non-null)"
        )
        return f"{self.metric_key}: HTTP {self.status} — {detail}"


def _probe(client: ApiClient, person_id: str, start: str, end: str, metric_key: str) -> MetricProbe:
    """Ask one metric for one person over one period, and describe the answer.

    Shape problems are RECORDED rather than raised, so the check above can
    report every key at once. A gate that stopped at the first unreadable body
    would hide the two keys that might have answered perfectly well.
    """
    response = client.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [person_id]},
            "period": {"from": start, "to": end},
            "metrics": [
                {
                    "metric_key": metric_key,
                    # Both views, because they fail differently: a period value
                    # can be non-null while the timeseries is empty (a bucketing
                    # or window bug), and a timeseries can carry points while the
                    # period scalar is null (an aggregation bug). A dashboard
                    # renders both.
                    "views": [{"view": "period"}, {"view": "timeseries"}],
                }
            ],
        },
    )
    if response.status_code != 200:
        return MetricProbe(
            metric_key=metric_key,
            status=response.status_code,
            note=f"not answered: {response.text[:200]}",
        )

    body = response.json()
    if not isinstance(body, dict):
        return MetricProbe(
            metric_key=metric_key,
            status=200,
            note=f"the body is not a JSON object: {response.text[:200]}",
        )

    metrics = _objects(body.get("metrics"))
    answered = [str(entry.get("metric_key", "")) for entry in metrics]
    if answered != [metric_key]:
        return MetricProbe(
            metric_key=metric_key,
            status=200,
            note=(
                f"asked for one metric and the response answered for {answered}: "
                f"{response.text[:200]}"
            ),
        )

    by_view = {str(view.get("view", "")): view for view in _objects(metrics[0].get("views"))}

    period = by_view.get("period")
    if period is None:
        return MetricProbe(
            metric_key=metric_key,
            status=200,
            note=f"no period view came back; views present: {sorted(by_view)}",
        )
    period_values = tuple(
        _number(entry.get("value"))
        for entry in _objects(period.get("values"))
        if str(entry.get("entity_id", "")) == person_id
    )
    if not period_values:
        return MetricProbe(
            metric_key=metric_key,
            status=200,
            note=f"the period view carried no value for the person asked about ({person_id})",
        )

    timeseries = by_view.get("timeseries")
    if timeseries is None:
        return MetricProbe(
            metric_key=metric_key,
            status=200,
            note=f"no timeseries view came back; views present: {sorted(by_view)}",
            period_values=period_values,
        )
    series = _objects(timeseries.get("series"))
    points = [point for entry in series for point in _objects(entry.get("points"))]

    return MetricProbe(
        metric_key=metric_key,
        status=200,
        period_values=period_values,
        series=len(series),
        points=len(points),
        non_null_points=sum(1 for point in points if _number(point.get("value")) is not None),
    )


__all__: Sequence[str] = ("SEED_GUARANTEED_METRIC_KEYS",)
