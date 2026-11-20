local cjson = require("cjson.safe")

local _M = {}

-- Canonical error `type` URNs (toolkit CanonicalError::gts_type()).
local TYPE_UNAUTHENTICATED =
    "gts://gts.cf.core.errors.err.v1~cf.core.err.unauthenticated.v1~"
local TYPE_SERVICE_UNAVAILABLE =
    "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~"

local RETRY_AFTER = 5

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

function _M.bearer_unauthorized(resource_metadata_url, scope)
    local required_scope = scope or "mcp:query"
    local challenge = 'Bearer scope="' .. required_scope .. '"'
    if resource_metadata_url then
        challenge = 'Bearer resource_metadata="' .. resource_metadata_url
            .. '", scope="' .. required_scope .. '"'
    end
    ngx.header["WWW-Authenticate"] = challenge
    return problem(
        401,
        TYPE_UNAUTHENTICATED,
        "Unauthenticated",
        "A valid bearer token is required.",
        { reason = "invalid_bearer" }
    )
end

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
