#!/usr/bin/env python3
"""In-network e2e driver for the gateway (gateway DESIGN 3.9-3.12; NGINX_BFF §D).

Runs inside the compose network (so `gateway` and `fakeidp` resolve by name and
the OIDC redirect flow works without URL rewriting). Uses only the stdlib.

Phases (argv[1]) are orchestrated by run-e2e.sh, which stops/starts containers
between them and shares the session cookie via /work/sid.txt:

  core          scenarios 1, 2, 5 + cache populate (authenticator + echo up)
  authdown      scenario 3/4: cache still serves; a fresh cookie fails closed
  revoked       scenario 3: after logout + cache expiry, the cookie is rejected
  upstreamdown  scenario 4: exchange succeeds, dead upstream yields 502
"""

import base64
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

GW = "http://gateway:8080"
ROUTES = ["/api/v1/analytics", "/api/v1/identity"]
SID_FILE = "/work/sid.txt"

_failures = 0


def check(name, ok, detail=""):
    global _failures
    if ok:
        print(f"PASS: {name}")  # noqa: T201 -- e2e driver: PASS/FAIL output is the point
    else:
        _failures += 1
        print(f"FAIL: {name} {detail}")  # noqa: T201


def req(url, headers=None, method="GET"):
    """One request, redirects NOT followed. Returns (status, headers, body)."""
    r = urllib.request.Request(url, headers=headers or {}, method=method)

    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *a, **k):
            return None

    opener = urllib.request.build_opener(NoRedirect)

    # HTTP header names are case-insensitive, but the casing on the wire varies
    # by source (Lua-set vs proxied-through), so normalize keys to lowercase.
    def lower(headers):
        return {k.lower(): v for k, v in headers.items()}

    try:
        resp = opener.open(r, timeout=15)
        return resp.status, lower(resp.headers), resp.read()
    except urllib.error.HTTPError as e:
        return e.code, lower(e.headers), e.read()


def login():
    """Drive the OIDC code flow through the gateway; return the __Host-sid value."""
    _, h, _ = req(f"{GW}/auth/login?return_to=/")
    authorize = h["location"]
    _, h, _ = req(authorize)
    callback = h["location"]  # absolute http://gateway:8080/auth/callback?code&state
    status, h, _ = req(callback)
    if status != 302:
        raise SystemExit(f"callback expected 302, got {status}: {h}")
    set_cookie = h.get("set-cookie", "")
    for part in set_cookie.split(";"):
        part = part.strip()
        if part.startswith("__Host-sid="):
            return part[len("__Host-sid=") :]
    raise SystemExit(f"no __Host-sid in Set-Cookie: {set_cookie!r}")


def b64url_json(segment):
    segment += "=" * (-len(segment) % 4)
    return json.loads(base64.urlsafe_b64decode(segment))


def phase_core():
    # Scenario 1: no cookie -> 401 + WWW-Authenticate on EVERY configured route.
    for route in ROUTES:
        status, h, body = req(f"{GW}{route}/x")
        check(f"no-cookie 401 on {route}", status == 401, f"got {status}")
        check(f"WWW-Authenticate on {route}", "www-authenticate" in h)
    # Canonical problem-details body (toolkit Problem shape) on the 401.
    prob = json.loads(body)
    check("401 is application/problem+json", "problem+json" in h.get("content-type", ""))
    check(
        "401 type is canonical unauthenticated URN",
        prob.get("type") == "gts://gts.cf.core.errors.err.v1~cf.core.err.unauthenticated.v1~",
        prob.get("type"),
    )
    check("401 title/status canonical", prob.get("title") == "Unauthenticated" and prob.get("status") == 401, str(prob))

    # Scenario 2: login -> cookie -> /api 200 with a verifiable JWT injected, the
    # session cookie stripped, a forged inbound Authorization replaced, and a
    # unique UUIDv7 correlation id per request (R3 poisoned-header snapshot).
    sid = login()
    Path(SID_FILE).write_text(sid)

    hdrs = {"Cookie": f"__Host-sid={sid}; keep=1", "Authorization": "Bearer FORGED", "X-Correlation-Id": "forged-corr"}
    status, _, body = req(f"{GW}/api/v1/analytics/data", headers=hdrs)
    check("authed request 200", status == 200, f"got {status}")
    echoed = json.loads(body)["headers"]

    auth = echoed.get("authorization", "")
    check("forged Authorization replaced by Bearer JWT", auth.startswith("Bearer ") and auth != "Bearer FORGED", auth)
    jwt = auth.split(" ", 1)[1] if " " in auth else ""
    parts = jwt.split(".")
    check("injected JWT is well-formed (3 segments)", len(parts) == 3)
    if len(parts) == 3:
        header = b64url_json(parts[0])
        claims = b64url_json(parts[1])
        check("JWT alg is ES256", header.get("alg") == "ES256", str(header))
        check("JWT carries sub", "sub" in claims, str(claims))
    check(
        "__Host-sid stripped from upstream Cookie",
        "__Host-sid" not in echoed.get("cookie", ""),
        echoed.get("cookie", ""),
    )
    check("non-gateway cookie preserved", "keep=1" in echoed.get("cookie", ""))
    corr = echoed.get("x-correlation-id", "")
    check("correlation id not the forged one", corr and corr != "forged-corr", corr)
    check(
        "correlation id is a UUIDv7",
        re.match(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$", corr) is not None,
        corr,
    )

    # Verifiable: JWKS is served DIRECTLY by the authenticator (the key issuer),
    # not fronted by the gateway. Downstream services fetch it there.
    status, _, body = req("http://authenticator:8083/.well-known/jwks.json")
    check("JWKS served by the authenticator", status == 200, f"got {status}")
    check("JWKS has keys", "keys" in json.loads(body))

    # Unique UUIDv7 per request.
    corrs = set()
    for _ in range(3):
        _, _, b = req(f"{GW}/api/v1/analytics/data", headers={"Cookie": f"__Host-sid={sid}"})
        corrs.add(json.loads(b)["headers"].get("x-correlation-id"))
    check("3 requests -> 3 unique correlation ids", len(corrs) == 3, str(corrs))

    # Scenario 5: SPA passthrough and the internal surface.
    status, _, body = req(f"{GW}/")
    check("SPA passthrough on /", status == 200 and json.loads(body)["path"] == "/", f"{status}")
    check("/internal/anything -> 404", req(f"{GW}/internal/anything")[0] == 404)
    check("unmatched /api -> 404", req(f"{GW}/api/v1/nope")[0] == 404)


def phase_authdown():
    # Scenario 3/4: with the authenticator stopped, a cached cookie is still
    # served from the per-pod lua_shared_dict, while a cold cookie fails closed.
    sid = Path(SID_FILE).read_text().strip()
    status, _, _ = req(f"{GW}/api/v1/analytics/x", headers={"Cookie": f"__Host-sid={sid}"})
    check("cached cookie still 200 while authenticator down", status == 200, f"got {status}")

    status, h, body = req(f"{GW}/api/v1/analytics/x", headers={"Cookie": "__Host-sid=cold-never-seen"})
    check("cold cookie -> 503 fail closed", status == 503, f"got {status}")
    check("503 carries Retry-After", "retry-after" in h)
    prob = json.loads(body)
    check(
        "503 type is canonical service_unavailable URN",
        prob.get("type") == "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~",
        prob.get("type"),
    )
    check("503 context has retry_after_seconds", prob.get("context", {}).get("retry_after_seconds") == 5, str(prob))


def phase_revoked():
    # Scenario 3: after logout and cache expiry, the session is rejected.
    sid = Path(SID_FILE).read_text().strip()
    req(f"{GW}/auth/logout", headers={"Cookie": f"__Host-sid={sid}"}, method="POST")
    status, _, _ = req(f"{GW}/api/v1/analytics/x", headers={"Cookie": f"__Host-sid={sid}"})
    check("revoked session -> 401 within cache max-age", status == 401, f"got {status}")


def phase_upstreamdown():
    # Scenario 4: exchange succeeds (authenticator up) but the upstream is dead.
    sid = login()
    status, _, _ = req(f"{GW}/api/v1/analytics/x", headers={"Cookie": f"__Host-sid={sid}"})
    check("dead upstream -> 502", status == 502, f"got {status}")


PHASES = {"core": phase_core, "authdown": phase_authdown, "revoked": phase_revoked, "upstreamdown": phase_upstreamdown}

if __name__ == "__main__":
    phase = sys.argv[1] if len(sys.argv) > 1 else "core"
    PHASES[phase]()
    print(f"--- phase '{phase}': {'OK' if _failures == 0 else str(_failures) + ' FAILURES'} ---")  # noqa: T201
    sys.exit(1 if _failures else 0)
