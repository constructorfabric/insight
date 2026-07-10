-- errors.lua -- canonical problem-details error shaping (gateway DESIGN 3.11, 3.12).
--
-- Emits the exact wire shape the toolkit's canonical errors produce
-- (cf-gears-toolkit-canonical-errors `Problem`, RFC 9457): the `type` is the
-- canonical `gts://...` URN, plus `title`/`status`/`detail`/`context`. One error
-- format from the edge to the gear, so a client parses gateway and service
-- errors identically.
--
-- The access phase shapes the two fail-closed cases: 401 (no session /
-- authenticator refused) and 503 (authenticator unreachable / timed out / 5xx).

local cjson = require("cjson.safe")

local _M = {}

-- Canonical error `type` URNs (toolkit CanonicalError::gts_type()).
local TYPE_UNAUTHENTICATED =
    "gts://gts.cf.core.errors.err.v1~cf.core.err.unauthenticated.v1~"
local TYPE_SERVICE_UNAVAILABLE =
    "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~"

-- Bounded Retry-After (seconds) for the unavailable case.
local RETRY_AFTER = 5

-- Emit a canonical Problem and terminate the request. `context` is the
-- category-specific object (UnauthenticatedV1 / ServiceUnavailableV1); omitted
-- optional members (instance, trace_id, resource_*) are simply absent, matching
-- the toolkit's `skip_serializing_if = None`.
local function problem(status, ptype, title, detail, context)
    ngx.status = status
    ngx.header["Content-Type"] = "application/problem+json"
    ngx.say(cjson.encode({
        type = ptype,
        title = title,
        status = status,
        detail = detail,
        context = context,
    }))
    return ngx.exit(status)
end

--- 401 Unauthenticated: no session / authenticator refused. The WWW-Authenticate
--- header is the SPA's login-redirect signal (G9).
function _M.unauthorized()
    ngx.header["WWW-Authenticate"] = 'Session realm="insight"'
    return problem(
        401,
        TYPE_UNAUTHENTICATED,
        "Unauthenticated",
        "No valid session; authenticate at /auth/login.",
        { reason = "no_session" }
    )
end

--- 503 Service Unavailable: authenticator unreachable, timed out, or 5xx --
--- fail closed, shaped, with Retry-After.
function _M.unavailable(detail)
    ngx.header["Retry-After"] = tostring(RETRY_AFTER)
    return problem(
        503,
        TYPE_SERVICE_UNAVAILABLE,
        "Service Unavailable",
        detail or "The authentication service is unavailable.",
        { retry_after_seconds = RETRY_AFTER }
    )
end

return _M
