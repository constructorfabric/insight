-- errors.lua -- RFC 9457 problem-details error shaping (gateway DESIGN 3.11, 3.12).
--
-- The access phase sees the exchange outcome directly and shapes the two
-- fail-closed cases: 401 (authenticator refused / no session) and 503
-- (authenticator unreachable / timed out / 5xx). Same problem+json format the
-- toolkit's canonical errors produce -- one shape from the edge to the gear.

local cjson = require("cjson.safe")

local _M = {}

-- Bounded Retry-After (seconds) for the unavailable case; matches the DESIGN
-- sketch's 503 body.
local RETRY_AFTER = 5

local function respond(status, title, detail, extra)
    ngx.status = status
    ngx.header["Content-Type"] = "application/problem+json"
    local body = {
        type = "about:blank",
        title = title,
        status = status,
    }
    if detail then
        body.detail = detail
    end
    if extra then
        for k, v in pairs(extra) do
            body[k] = v
        end
    end
    ngx.say(cjson.encode(body))
    return ngx.exit(status)
end

--- 401: no session / authenticator refused. Never cached upstream of here; the
--- WWW-Authenticate header and the login URL are the SPA contract (G9).
function _M.unauthorized()
    ngx.header["WWW-Authenticate"] = 'Session realm="insight"'
    return respond(401, "unauthenticated", nil, { login = "/auth/login" })
end

--- 503: authenticator unreachable, timed out, or 5xx -- fail closed, shaped.
function _M.unavailable(detail)
    ngx.header["Retry-After"] = tostring(RETRY_AFTER)
    return respond(503, "auth_unavailable", detail, { retry_after_seconds = RETRY_AFTER })
end

return _M
