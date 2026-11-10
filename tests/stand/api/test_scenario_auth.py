from __future__ import annotations

import base64
import hashlib
import json
import os
import re
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import parse_qs, urlencode, urlsplit
from uuid import UUID

import jwt
import pytest
from insight_stand import (
    ASSERTION_TYPE,
    SESSION_COOKIE_NAME,
    ApiClient,
    ApiResponse,
    LoginSession,
    Manifest,
    Person,
    PersonaSession,
    ServiceTokenSession,
    default_token_url,
    identity_path,
    persona_password,
    wait_until,
)
from insight_stand.stand import PUBLISHED_HOST, candidate_env_files, parse_env_file
from pydantic import BaseModel

from .schemas import PROBLEM_CONTENT_TYPE, MeResponse, ProblemDocument
from .schemas.authenticator_untyped import (
    AuthMe,
    CsrfToken,
    Jwk,
    Jwks,
    LoggedOut,
    OpenIdConfiguration,
    Refreshed,
    Revoked,
    ServiceToken,
    SessionList,
)

pytestmark = pytest.mark.security

_REPO_ROOT = Path(__file__).resolve().parents[3]

# The gateway caches a session's JWT for authz_cache_max_age_seconds: a revoked
# session that already reached /api keeps reaching it that long.
GATEWAY_AUTHZ_CACHE_S = 30

JWT_AUDIENCE = "internal-services"
API_PROBE = identity_path("/v1/me")
_SESSION_COOKIE = re.compile(rf"{re.escape(SESSION_COOKIE_NAME)}=([^;]*)")


@dataclass
class CookieSession:
    value: str

    def headers(self) -> dict[str, str]:
        return {"Cookie": f"{SESSION_COOKIE_NAME}={self.value}"}


@dataclass
class Device:
    person: Person
    session: CookieSession
    client: ApiClient
    csrf_token: str | None = None

    def csrf(self) -> dict[str, str]:
        if self.csrf_token is None:
            self.csrf_token = _ok(self.client.get("/auth/csrf"), CsrfToken).csrf_token
        return {"X-CSRF-Token": self.csrf_token}


@pytest.fixture
def sign_in(
    stand_manifest: Manifest,
    stand_base_url: str,
    session_for: Callable[[str], PersonaSession],
) -> Iterator[Callable[[str], Device]]:
    devices: list[Device] = []
    signed_in: set[str] = set()

    def _sign_in(fixture_name: str) -> Device:
        person = stand_manifest.fixture(fixture_name)
        login = LoginSession(
            base_url=stand_base_url,
            email=person.email,
            password=persona_password(person.email),
        )
        cookie = login.headers()["Cookie"].removeprefix(f"{SESSION_COOKIE_NAME}=")

        session = CookieSession(cookie)
        device = Device(person, session, ApiClient(base_url=stand_base_url, session=session))
        devices.append(device)
        signed_in.add(fixture_name)
        return device

    yield _sign_in

    for device in devices:
        if device.client.get(API_PROBE).status_code != 200:
            continue
        if device.csrf_token is None:
            csrf = device.client.get("/auth/csrf")
            if csrf.status_code != 200:
                continue
            device.csrf_token = csrf.parse(CsrfToken).csrf_token
        device.client.post("/auth/logout", headers=device.csrf())

    # Revoking every session of a persona also ends the suite's cached one.
    for fixture_name in sorted(signed_in):
        session_for(fixture_name).session.login()


def _ok[T: BaseModel](response: ApiResponse, model: type[T]) -> T:
    assert response.status_code == 200, (
        f"{response.url} answered {response.status_code}: {response.text[:300]}"
    )
    return response.parse(model)


def _problem(response: ApiResponse, status: int) -> ProblemDocument:
    assert response.status_code == status, (
        f"{response.url} answered {response.status_code}, expected {status}: {response.text[:300]}"
    )
    assert response.content_type == PROBLEM_CONTENT_TYPE, response.content_type
    return response.parse(ProblemDocument)


def _assert_csrf_refused(response: ApiResponse) -> None:
    reason = _problem(response, 403).context.get("reason")
    assert reason == "csrf_token_required", reason


def _assert_api_refused(device: Device) -> None:
    reason = _problem(device.client.get(API_PROBE), 401).context.get("reason")
    assert reason == "no_session", reason


def _assert_api_refused_once_the_cache_lapses(device: Device) -> None:
    wait_until(
        lambda: device.client.get(API_PROBE).status_code == 401,
        timeout_s=GATEWAY_AUTHZ_CACHE_S + 15,
        interval_s=1.0,
        description=f"the gateway to refuse {device.person.email}'s signed-out session at /api",
    )
    _assert_api_refused(device)


def _session_cookie(response: ApiResponse) -> str:
    match = _SESSION_COOKIE.search(response.headers.get("set-cookie", ""))
    assert match, f"{response.url} set no {SESSION_COOKIE_NAME}: {response.headers}"
    return match.group(1)


def _sessions(device: Device) -> tuple[UUID, set[UUID]]:
    listed = _ok(device.client.get("/auth/sessions"), SessionList).sessions
    current = [session.session_id for session in listed if session.current]
    assert len(current) == 1, f"{device.person.email} lists {len(current)} current sessions"
    return current[0], {session.session_id for session in listed}


def _authenticator_url() -> str:
    env_file = next(
        (path for path in candidate_env_files(_REPO_ROOT, os.environ) if path.is_file()), None
    )
    port = parse_env_file(env_file).get("AUTHENTICATOR_PORT", "") if env_file else ""
    return f"http://{PUBLISHED_HOST}:{port.strip() or '8083'}"


def _thumbprint(key: Jwk) -> str:
    members = {"crv": key.crv, "kty": key.kty, "x": key.x, "y": key.y}
    canonical = json.dumps(members, separators=(",", ":"), sort_keys=True).encode()
    return base64.urlsafe_b64encode(hashlib.sha256(canonical).digest()).rstrip(b"=").decode()


@pytest.mark.requires_seed("dev_lead")
def test_a_lead_signs_in_stays_signed_in_and_signing_out_closes_the_api(
    api_client: ApiClient, sign_in: Callable[[str], Device]
) -> None:
    login = api_client.get("/auth/login")
    assert login.status_code == 302, f"/auth/login answered {login.status_code}"
    authorize = urlsplit(login.headers["location"])
    query = parse_qs(authorize.query)
    assert query["response_type"] == ["code"]
    assert query["code_challenge_method"] == ["S256"]
    assert urlsplit(query["redirect_uri"][0]).path == "/auth/callback"

    lead = sign_in("dev_lead")
    me = _ok(lead.client.get("/auth/me"), AuthMe)
    assert str(me.user) == lead.person.uuid
    assert lead.csrf() == {"X-CSRF-Token": me.csrf_token}

    through_gateway = _ok(lead.client.get(API_PROBE), MeResponse)
    assert str(through_gateway.person_id) == lead.person.uuid

    _assert_csrf_refused(lead.client.post("/auth/refresh"))

    refresh = lead.client.post("/auth/refresh", headers=lead.csrf())
    refreshed = _ok(refresh, Refreshed)
    assert refreshed.expires_at >= me.expires_at

    previous_cookie = lead.session.value
    lead.session.value = _session_cookie(refresh)
    assert lead.session.value not in ("", previous_cookie)

    after_refresh = _ok(lead.client.get("/auth/me"), AuthMe)
    assert after_refresh.user == me.user
    assert after_refresh.expires_at == refreshed.expires_at

    stale = ApiClient(base_url=lead.client.base_url, session=CookieSession(previous_cookie))
    wait_until(
        lambda: stale.get("/auth/me").status_code == 401,
        timeout_s=5,
        interval_s=0.3,
        description="the cookie replaced by /auth/refresh to stop resolving",
    )
    assert _ok(lead.client.get(API_PROBE), MeResponse).person_id == through_gateway.person_id

    _assert_csrf_refused(lead.client.post("/auth/logout"))
    assert lead.client.get("/auth/me").status_code == 200

    csrf = lead.csrf()
    logout = lead.client.post("/auth/logout", headers=csrf)
    signed_out = _ok(logout, LoggedOut)
    assert signed_out.rp_logout_url is not None
    idp_logout = urlsplit(signed_out.rp_logout_url)
    assert idp_logout.netloc == authorize.netloc
    assert "id_token_hint" in parse_qs(idp_logout.query)
    assert _session_cookie(logout) == ""
    assert "Max-Age=0" in logout.headers["set-cookie"]

    assert lead.client.get("/auth/me").status_code == 401
    assert lead.client.post("/auth/refresh", headers=csrf).status_code == 401
    _assert_api_refused_once_the_cache_lapses(lead)


@pytest.mark.requires_seed("hr_ic", "dev_lead")
def test_signing_out_one_device_spares_the_other_and_signing_out_everywhere_ends_all(
    sign_in: Callable[[str], Device],
) -> None:
    laptop = sign_in("hr_ic")
    phone = sign_in("hr_ic")
    someone_else = sign_in("dev_lead")

    laptop_id, seen_from_laptop = _sessions(laptop)
    phone_id, seen_from_phone = _sessions(phone)
    someone_elses_id, _ = _sessions(someone_else)
    assert laptop_id != phone_id
    assert phone_id in seen_from_laptop
    assert laptop_id in seen_from_phone
    assert someone_elses_id not in seen_from_laptop

    _assert_csrf_refused(laptop.client.delete(f"/auth/sessions/{phone_id}"))
    assert phone.client.get("/auth/me").status_code == 200

    revoked = laptop.client.delete(f"/auth/sessions/{phone_id}", headers=laptop.csrf())
    assert _ok(revoked, Revoked).revoked == 1

    assert phone.client.get("/auth/me").status_code == 401
    _assert_api_refused(phone)
    _ok(laptop.client.get(API_PROBE), MeResponse)
    assert phone_id not in _sessions(laptop)[1]

    again = laptop.client.delete(f"/auth/sessions/{phone_id}", headers=laptop.csrf())
    _problem(again, 404)

    trespass = laptop.client.delete(f"/auth/sessions/{someone_elses_id}", headers=laptop.csrf())
    _problem(trespass, 404)

    tablet = sign_in("hr_ic")
    everywhere = laptop.client.delete("/auth/sessions", headers=laptop.csrf())
    assert _ok(everywhere, Revoked).revoked >= 2

    assert laptop.client.get("/auth/me").status_code == 401
    assert tablet.client.get("/auth/me").status_code == 401
    _assert_api_refused(tablet)
    _ok(someone_else.client.get(API_PROBE), MeResponse)


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("hr_ic", "admin_operator")
def test_a_service_signs_a_person_out_everywhere_and_nobody_else(
    sign_in: Callable[[str], Device],
    service_session: ServiceTokenSession,
    stand_base_url: str,
) -> None:
    laptop = sign_in("hr_ic")
    phone = sign_in("hr_ic")
    operator = sign_in("admin_operator")
    _ok(operator.client.get(API_PROBE), MeResponse)
    every_session = f"/auth/admin/users/{laptop.person.uuid}/sessions"

    by_a_person = laptop.client.delete(every_session, headers=laptop.csrf())
    assert _problem(by_a_person, 401).context.get("reason") == "MISSING_BEARER"
    assert laptop.client.get("/auth/me").status_code == 200

    service = ApiClient(base_url=stand_base_url, session=service_session)
    assert _ok(service.delete(every_session), Revoked).revoked >= 2

    for device in (laptop, phone):
        assert device.client.get("/auth/me").status_code == 401
        _assert_api_refused(device)
    assert operator.client.get("/auth/me").status_code == 200
    _ok(operator.client.get(API_PROBE), MeResponse)


@pytest.mark.requires_service_principal
def test_a_downstream_service_finds_the_signing_keys_and_they_verify_an_issued_token(
    service_session: ServiceTokenSession, stand_manifest: Manifest
) -> None:
    form = {
        "grant_type": "client_credentials",
        "client_assertion_type": ASSERTION_TYPE,
        "client_assertion": service_session.assertion(),
        "tenant_id": stand_manifest.tenant,
    }
    issued = ApiClient(base_url=default_token_url(), edge_fronted=False).post(
        "/internal/token",
        content=urlencode(form),
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    token = _ok(issued, ServiceToken)

    authenticator = ApiClient(base_url=_authenticator_url(), edge_fronted=False)
    discovery = _ok(authenticator.get("/.well-known/openid-configuration"), OpenIdConfiguration)
    assert discovery.jwks_uri == f"{discovery.issuer}/.well-known/jwks.json"
    assert "ES256" in discovery.id_token_signing_alg_values_supported

    keys = _ok(authenticator.get(urlsplit(discovery.jwks_uri).path), Jwks).keys
    assert [key.kid for key in keys] == [_thumbprint(key) for key in keys]

    kid = jwt.get_unverified_header(token.access_token).get("kid")
    signing = next((key for key in keys if key.kid == kid), None)
    assert signing is not None, f"the token's kid {kid!r} is not in {discovery.jwks_uri}"

    claims = jwt.decode(
        token.access_token,
        jwt.PyJWK(signing.model_dump()).key,
        algorithms=["ES256"],
        audience=JWT_AUDIENCE,
        issuer=discovery.issuer,
    )
    assert claims["tenant_id"] == stand_manifest.tenant
    assert claims["sub_type"] == "service"
    assert "service" in claims["roles"].split()
    assert claims["exp"] - claims["iat"] == token.expires_in
