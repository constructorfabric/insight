---
status: accepted
date: 2026-07-07
---

# ADR-0001: Pure `access_by_lua` Exchange Instead of the `auth_request` Directive

**ID**: `cpt-insightspec-adr-gw-0001-access-by-lua-over-auth-request`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A -- pure `access_by_lua` (chosen)](#option-a----pure-access_by_lua-chosen)
  - [Option B -- `auth_request` + `proxy_cache`](#option-b----auth_request--proxy_cache)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

The gateway authenticates every `/api/*` request by exchanging the `__Host-sid` session
cookie for the session-linked gateway JWT, caching the result per pod (`DD-GW-03`: a
`lua_shared_dict` keyed by the cookie token, TTL driven by the authenticator's
`Cache-Control`). Two nginx mechanisms can carry that exchange, and the step-01 DESIGN prose
mixes them: it commits to a Lua shared-memory cache (`DD-GW-03`, section 3.11) yet describes
the subrequest itself throughout as the stock `auth_request` directive (section 3.10, 3.12,
the config sketch).

Those two do not compose. The `auth_request` directive fires on **every** matched request by
construction -- there is no supported way to *skip* it when a prior `lua_shared_dict` lookup
already holds the JWT. So a shared-dict cache implies the miss-path subrequest is issued
**from Lua** (a `lua-resty-http` cosocket call, or `ngx.location.capture` to an internal
proxy location), not by the `auth_request` directive. Which mechanism is the implementation
baseline for the gateway (step 05), and does the full edge contract actually hold on it?

This was de-risked by a throwaway `docker compose` spike (EPIC #1583, step 02, #1585) before
any production code. This ADR records the outcome; the runnable harness was intentionally not
committed (it lives outside the repo -- see More Information).

## Decision Drivers

- `DD-GW-03` already requires a per-pod `lua_shared_dict` exchange cache; the mechanism must
  honor a Lua-gated cache hit without an unconditional subrequest.
- Per-request UUIDv7 `X-Correlation-Id` must be generated at the edge and must **not** be read
  from the cacheable exchange response (or it repeats for a whole cache window).
- Fail-closed error shaping must distinguish "authenticator refused" (401) from "unreachable"
  / "timed out" (503 + `Retry-After`), as RFC 9457 problem-details.
- The module must stay trivial and readable -- the gateway is the *commodity* half we chose
  nginx precisely to avoid hand-writing (`DD-GW-01`).
- The subrequest contract (section 3.10) must hold identically whichever mechanism wins, so a
  later reversal is cheap.

## Considered Options

- **Option A -- pure `access_by_lua`**: one access-phase Lua module does everything --
  `lua_shared_dict` lookup; on miss a `lua-resty-http` cosocket call to `/internal/authz`;
  parse `Cache-Control` and populate the dict; inject `Authorization`, strip `__Host-sid`,
  generate the UUIDv7; return 401/503 directly. No `auth_request` directive.
- **Option B -- `auth_request` + `proxy_cache`**: the stock forward-auth shape -- an
  `internal` location `proxy_pass`ing to the authenticator with `proxy_cache` +
  `proxy_cache_lock`, `auth_request_set` lifting `X-Gateway-Jwt`; Lua only for the UUIDv7
  (`set_by_lua`) and error shaping (`error_page`).

## Decision Outcome

**Chosen: Option A.** Both were built into the same spike gateway (`/api/*` = A, `/apib/*` = B)
and **both passed all nine edge proofs**, so correctness is not the differentiator -- Option B
remains the documented exit path (`DD-GW-03`, section 3.11) with an identical subrequest
contract. Option A wins on fit:

1. **UUIDv7 is native to A, contorted in B.** A generates it in the same access phase after
   the cache decision and sets it as a request header -- per-request by construction. B must
   use `set_by_lua` *and* keep the id out of the cacheable subrequest response.
2. **Error shaping is first-class in A.** A sees the exact subrequest outcome
   (connect-refused vs timeout vs non-200) and shapes 401 vs 503 directly. B collapses
   "unreachable" into nginx's bare 500 and reshapes it via `error_page 500` -- it works, but
   cannot tell refused from timed-out without extra machinery.
3. **One language owns the cache key and TTL.** A reads the authenticator's `max-age` and calls
   `dict:set(sid, jwt, ttl)`; the travel margin, LRU sizing, and `no-store`-on-401 live in a
   dozen lines of one module. B spreads the same semantics across `proxy_cache_valid`, the
   cookie `map`, and upstream headers.
4. **The whole edge is one readable straight-line module** (~70 lines of code) -- nothing like
   the Rust Router it replaces.
5. **Memory safety is free** (`DD-GW-03`): the fixed-size `lua_shared_dict` with native LRU
   cannot OOM the gateway. B's `proxy_cache` is disk-backed.

### Consequences

- The step-05 baseline is the Lua module of section 3.11, with the miss-path subrequest issued
  by `lua-resty-http` from the access phase. **The `auth_request` directive is not used**; the
  DESIGN's `auth_request` wording (sections 3.10, 3.12, the sketch) is reconciled to
  "access-phase Lua exchange" -- the subrequest *contract* is unchanged, only the mechanism.
- **A `resolver` directive becomes mandatory** (see More Information): `lua-resty-http`
  cosockets do their own DNS. This is the one real cost of A over B and it is a one-liner.
- The image must ship `lua-resty-http` (`opm`), which the base OpenResty image does not bundle.
- Option B stays fully documented as the zero-code exit if the Lua module ever becomes a
  burden; the subrequest contract is identical, so reversal touches only the gateway location
  blocks, never the authenticator.

### Confirmation

The spike proved all nine edge mechanics live against a three-container compose
(OpenResty gateway + a stub authenticator + `jmalloc/echo-server`). Evidence:

| # | Proof | Observed result |
|---|---|---|
| 1 | JWT injection / cookie strip / forged-`Authorization` replaced | upstream got `Authorization: Bearer <jwt>`; forged token gone; `__Host-sid` stripped at start/middle/end positions (no leak) |
| 2 | 401 path, no upstream call | absent cookie = fast 401, **0** authenticator calls; bad cookie = **1** call then 401; `echo` never reached in either |
| 3 | exchange cache honors `Cache-Control` | 100 identical requests in 10 s -> **1** authenticator call; 401 never cached (good cookie right after a 401 succeeds); entry expires after the 10 s `max-age` (call count 1 -> 2) |
| 4 | per-request UUIDv7 on cache hits | 3 cache-hit requests -> 3 distinct well-formed UUIDv7 `X-Correlation-Id` |
| 5 | WebSocket through the same path | authed upgrade -> `101` + frame echo; unauthenticated upgrade denied at the access phase (`401`, no socket) |
| 6 | 100 MB stream, `proxy_buffering off` | 104,857,600 bytes; TTFB ~5 ms << total; **0** files in `proxy_temp`; gateway RSS ~30 MB (not buffered) |
| 7 | reload under load | `ab -c30 -n30000` (no keepalive) + 4 live reloads -> **0** failed; a 20k sequential status-checked loop + 5 reloads -> `bad=0` |
| 8 | fail closed | authenticator down -> `503` + `Retry-After` problem-details (not a bare 500); upstream down -> `502`/`504` passthrough, distinct from the auth 503 |
| 9 | rate limit | `rate=60r/m burst=5 nodelay` -> 6x `200` then `429`, as configured |

The two failure modes that would hurt silently -- a cached 401, and caching past the
`Cache-Control` TTL -- are the e2e cases section 3.11 calls for; both are covered above.

## Pros and Cons of the Options

### Option A -- pure `access_by_lua` (chosen)

- Good: UUIDv7 and RFC 9457 error shaping are native to the access phase; cache key and TTL
  live in one language; the module is a readable straight-line chain; fixed-size shm cannot OOM.
- Good: cosockets are confirmed usable in the access phase (they are forbidden only in
  `set_by_lua` / `header_filter` / `body_filter` / `log_by_lua` / `init_by_lua`).
- Bad: requires a `resolver` directive for cosocket DNS, and `lua-resty-http` must be installed
  into the image (`opm get`, which pulls `perl` + `curl`).

### Option B -- `auth_request` + `proxy_cache`

- Good: zero custom code for the cache and subrequest; nginx resolves upstream names via core
  config, so no `resolver` is needed; the documented pattern for caching token introspection.
- Bad: cannot be gated by a `lua_shared_dict` hit (the directive always fires), so it is not
  actually the `DD-GW-03` cache; UUIDv7 and error shaping are awkward; the cookie-strip `map`
  leaves a cosmetic leading `"; "` when `__Host-sid` was first in the header; exchange
  semantics are spread across three mechanisms.

## More Information

Gotchas recorded for step-05 implementation:

- **Cosocket DNS needs `resolver`.** `lua-resty-http`'s `request_uri` resolves the hostname
  itself and fails with `no resolver defined to resolve "authenticator"` until the `http` block
  has `resolver <dns> ipv6=off;`. In-cluster use the CoreDNS/kube-dns service IP; the
  configurator (`DD-GW-02`) should emit it as a Helm value. nginx's own `upstream`/`proxy_pass`
  (all of Option B) needs none of this.
- **`lua-resty-http` is not bundled** in `openresty/openresty:*-alpine`; install with
  `opm get ledgetech/lua-resty-http`, which additionally needs `apk add perl curl`. Bake into
  the gateway image (a `configurator`/chart concern, not runtime).
- **`ab -k` miscounts reloads.** With keepalive, `ab` reports `Connection reset by peer` on
  reload because nginx closes idle keepalive sockets gracefully; it is a benchmark-tool
  artifact, not a dropped request. Verify reload safety without keepalive or by checking status
  codes directly.
- **`ngx.location.capture` is the no-dependency alternative** to `lua-resty-http` for the miss
  path (an internal `proxy_pass` location, resolved by nginx core, no `resolver` needed). Not
  chosen for the spike because cosockets exercise the access-phase constraint the design leans
  on, but it is a valid fallback if the `lua-resty-http` dependency is ever unwanted.

The throwaway spike harness (compose, `nginx.conf`, `gateway.lua`, stub authenticator, WS
client, `run-proofs.sh` reproducing every transcript above) was **not committed** -- it is
disposable and would only rot in the product tree. It is preserved outside the repo for
re-runs; the module shape it validated is captured in DESIGN section 3.11.

## Traceability

- [`cpt-insightspec-design-gateway`](../../DESIGN.md#dd-gw-03-gateway-side-exchange-cache-in-lua-shared-memory) -- `DD-GW-03`, the exchange-cache decision this ADR implements
- [`cpt-insightspec-design-gateway-lua-module`](../../DESIGN.md#311-lua-module) -- the module baseline chosen here
- [Subrequest Contract](../../DESIGN.md#310-subrequest-contract) and [Failure Handling](../../DESIGN.md#312-failure-handling) -- unchanged by this decision (mechanism only)
- [`cpt-insightspec-contract-auth-authz-exchange`](../../../authenticator/PRD.md#72-external-integration-contracts) -- the authenticator side of the exchange
