-- gateway.lua -- the access-phase cookie-to-JWT exchange (ADR-0001 Option A;
-- gateway DESIGN 3.10, 3.11).
--
-- A straight-line chain, deliberately trivial:
--   dict hit  -> inject Authorization, done.
--   dict miss -> lua-resty-http cosocket to the authenticator /internal/authz,
--                cache per the response Cache-Control (never a non-200), inject.
-- The auth_request directive is deliberately NOT used: it cannot be skipped on a
-- lua_shared_dict hit, so the miss-path subrequest is issued from Lua, which is
-- why the http block carries a `resolver` (cosockets do their own DNS).

local http = require("resty.http")
local uuid = require("resty.uuid")
local errors = require("errors")

local _M = {}

local CACHE = "jwt_cache"

-- Populated by init() from the generated init_by_lua_block.
local cfg = {
    authz_url = nil,
    authz_connect_timeout_ms = 2000,
    authz_read_timeout_ms = 2000,
}

function _M.init(opts)
    cfg.authz_url = opts.authz_url
    cfg.authz_connect_timeout_ms = opts.authz_connect_timeout_ms or cfg.authz_connect_timeout_ms
    cfg.authz_read_timeout_ms = opts.authz_read_timeout_ms or cfg.authz_read_timeout_ms
end

-- Extract the __Host-sid value from the Cookie header. The stock $cookie_*
-- variables cannot address the dash in the name, hence the manual match.
local function read_sid(cookie)
    if not cookie then
        return nil
    end
    return string.match(cookie, "__Host%-sid=([^;]+)")
end

-- Return the Cookie header with the gateway session cookie removed, at any
-- position; nil when nothing else remains. The session cookie never travels
-- upstream (DESIGN 3.9 item 3).
local function strip_cookie(cookie)
    if not cookie then
        return nil
    end
    local kept = {}
    for pair in string.gmatch(cookie, "[^;]+") do
        local trimmed = string.gsub(pair, "^%s*(.-)%s*$", "%1")
        if trimmed ~= "" and not string.find(trimmed, "^__Host%-sid=") then
            kept[#kept + 1] = trimmed
        end
    end
    if #kept == 0 then
        return nil
    end
    return table.concat(kept, "; ")
end

-- Parse the exchange TTL from the authenticator's Cache-Control. no-store (or
-- any non-200, handled by the caller) yields nil -- a rejection is never cached.
local function parse_max_age(cache_control)
    if not cache_control or string.find(cache_control, "no%-store") then
        return nil
    end
    local v = string.match(cache_control, "max%-age=(%d+)")
    return v and tonumber(v) or nil
end

-- Fetch the JWT for a session, honoring the per-pod exchange cache. Returns the
-- bearer string on success, or nil plus a terminal responder (already sent) on
-- 401/503.
local function resolve_bearer(cookie, sid)
    local cache = ngx.shared[CACHE]
    local bearer = cache:get(sid)
    if bearer then
        return bearer
    end

    local httpc = http.new()
    httpc:set_timeouts(
        cfg.authz_connect_timeout_ms,
        cfg.authz_read_timeout_ms,
        cfg.authz_read_timeout_ms
    )
    local res, err = httpc:request_uri(cfg.authz_url, {
        method = "GET",
        headers = { ["Cookie"] = cookie },
    })
    if not res then
        return nil, function() return errors.unavailable(err) end
    end
    if res.status == 401 then
        return nil, errors.unauthorized -- no-store: never cached
    end
    if res.status ~= 200 then
        return nil, function() return errors.unavailable("authz status " .. tostring(res.status)) end
    end
    bearer = res.headers["X-Gateway-Jwt"]
    if not bearer or bearer == "" then
        return nil, function() return errors.unavailable("authz 200 without X-Gateway-Jwt") end
    end

    -- Fixed-size shm with native LRU: set(), never safe_set() (DESIGN 3.11).
    local ttl = parse_max_age(res.headers["Cache-Control"])
    if ttl and ttl > 0 then
        cache:set(sid, bearer, ttl)
    end
    return bearer
end

-- The access-phase entry point wired into every generated /api/ location.
function _M.exchange()
    local cookie = ngx.var.http_cookie
    local sid = read_sid(cookie)
    if not sid then
        return errors.unauthorized()
    end

    local bearer, deny = resolve_bearer(cookie, sid)
    if not bearer then
        return deny()
    end

    -- Hygiene (DESIGN 3.9): inject the JWT (replacing anything the browser sent),
    -- strip the session cookie, mint a fresh correlation id, and clear
    -- client-supplied forwarding headers so the gateway is their sole author.
    ngx.req.set_header("Authorization", bearer)

    local remaining = strip_cookie(cookie)
    if remaining then
        ngx.req.set_header("Cookie", remaining)
    else
        ngx.req.clear_header("Cookie")
    end

    local corr = uuid.generate_time_v7()
    ngx.req.set_header("X-Correlation-Id", corr)
    ngx.var.correlation_id = corr

    ngx.req.clear_header("X-Forwarded-For")
    ngx.req.clear_header("X-Forwarded-Proto")
    ngx.req.clear_header("X-Forwarded-Host")
end

return _M
