-- uuid7.lua -- per-request UUIDv7 for X-Correlation-Id (gateway DESIGN 3.11).
--
-- Generated in the access phase, per request. It is structurally never read
-- from a cached exchange response (an id from there would repeat for a whole
-- cache window). Layout per RFC 9562: 48-bit big-endian unix-ms timestamp,
-- version nibble 0111, variant bits 10, the rest random.
--
-- Why hand-rolled: no OpenResty/luarocks library ships UUIDv7 (RFC 9562, 2024).
-- lua-resty-jit-uuid does v3/v4/v5 only; bungle/lua-resty-uuid is libuuid FFI
-- (v1/v4). v4 would forfeit the time-ordering the DESIGN wants for correlation
-- ids, so a focused ~40-line v7 generator is the right call.

local resty_random = require("resty.random")

local _M = {}

local byte = string.byte
local format = string.format
local floor = math.floor

--- Return a lowercase 8-4-4-4-12 UUIDv7 string.
function _M.generate()
    local ms = floor(ngx.now() * 1000)

    -- 16 random bytes; the timestamp and the version/variant nibbles overwrite
    -- the relevant ones. Prefer the strong source, fall back if it is starved.
    local rnd = resty_random.bytes(16, true) or resty_random.bytes(16)
    local b = { byte(rnd, 1, 16) }

    -- 48-bit millisecond timestamp, big-endian, into bytes 1..6.
    b[1] = floor(ms / 0x10000000000) % 0x100
    b[2] = floor(ms / 0x100000000) % 0x100
    b[3] = floor(ms / 0x1000000) % 0x100
    b[4] = floor(ms / 0x10000) % 0x100
    b[5] = floor(ms / 0x100) % 0x100
    b[6] = ms % 0x100

    -- version 7 in the high nibble of byte 7; variant 10 in the top two bits
    -- of byte 9.
    b[7] = 0x70 + (b[7] % 0x10)
    b[9] = 0x80 + (b[9] % 0x40)

    local hex = {}
    for i = 1, 16 do
        hex[i] = format("%02x", b[i])
    end
    return table.concat(hex, "", 1, 4)
        .. "-" .. table.concat(hex, "", 5, 6)
        .. "-" .. table.concat(hex, "", 7, 8)
        .. "-" .. table.concat(hex, "", 9, 10)
        .. "-" .. table.concat(hex, "", 11, 16)
end

return _M
