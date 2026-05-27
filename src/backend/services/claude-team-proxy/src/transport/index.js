// AuthedTransport contract — the seam between "HTTP server" and "actual
// way of talking to claude.ai".
//
// MVP: only PlaywrightTransport exists. The interface is overshaped on
// purpose so future swaps (curl_cffi, CloakBrowser CDP, mock for tests)
// drop in without server.js changes.
//
// Design rules:
//   1. fetch() does NOT throw on non-2xx HTTP. Pass status verbatim;
//      let caller distinguish "upstream said no" from "upstream
//      unreachable" (the latter throws).
//   2. init() may take tens of seconds (browser launch + CF challenge).
//      isReady() is the fast sync check used by /health.
//   3. Transport is single-use re: identity. One transport = one
//      sessionKey = one logged-in identity. Multi-tenant = multiple
//      transports = multiple pods (k8s replicas).
//   4. URL is full URL (https://...). Transport does not prepend a base.
//      Keeps the seam free of "knowledge of claude.ai".

/**
 * @typedef {Object} TransportResponse
 * @property {number} status - HTTP status code from upstream (1xx-5xx)
 * @property {string} body - response body as UTF-8 string
 * @property {Record<string, string>} headers - lowercased response headers.
 *   Sparse: implementations may not have all upstream headers if their
 *   underlying API does not expose them (page.evaluate(fetch) does not).
 */

/**
 * @typedef {Object} FetchOptions
 * @property {'GET'} [method='GET'] - HTTP method. MVP only supports GET.
 * @property {Record<string, string>} [headers] - additional request headers
 *   merged with transport defaults.
 */

/**
 * @typedef {Object} AuthedTransport
 *
 * @property {() => Promise<void>} init
 *   Establish authenticated session with upstream. Idempotent if called
 *   on already-initialised transport (no-op). Throws on auth failure or
 *   timeout — caller should treat the transport as dead.
 *
 * @property {(url: string, opts?: FetchOptions) => Promise<TransportResponse>} fetch
 *   Perform an authenticated request. Returns the response verbatim;
 *   non-2xx statuses are NOT thrown.
 *   Throws ONLY when the request could not complete (browser crashed,
 *   network error, transport-side timeout). In that case caller should
 *   return HTTP 502/504 to its own client.
 *
 * @property {() => boolean} isReady
 *   Synchronous: true if init() has successfully finished, false otherwise.
 *   Used by /health probe — must be cheap (no I/O).
 *
 * @property {() => Promise<void>} close
 *   Graceful shutdown. Releases all resources (browser processes,
 *   sockets). Safe to call on un-init'd transport.
 *
 * @property {string} kind - identifier for debug/health output, e.g.
 *   'playwright', 'mock'. Surfaced in /health response.
 *
 * @property {string} upstreamBaseUrl - the base URL this transport is
 *   bound to (e.g. "https://claude.ai"). Used by the HTTP server layer
 *   to build absolute upstream URLs from incoming request paths. Means
 *   the server never has to know the upstream hostname — it just asks
 *   the transport.
 */

import { createPlaywrightTransport } from './playwright.js';

/**
 * Factory. MVP returns playwright; future may switch on env var.
 *
 * @param {{
 *   kind?: 'playwright',
 *   sessionKey: string,
 *   upstreamBaseUrl: string,
 *   headless: boolean,
 *   startupTimeoutMs: number,
 *   fetchTimeoutMs: number,
 * }} config
 * @returns {AuthedTransport}
 */
export function createTransport(config) {
  const kind = config.kind ?? 'playwright';
  switch (kind) {
    case 'playwright':
      return createPlaywrightTransport({
        sessionKey: config.sessionKey,
        upstreamBaseUrl: config.upstreamBaseUrl,
        headless: config.headless,
        startupTimeoutMs: config.startupTimeoutMs,
        fetchTimeoutMs: config.fetchTimeoutMs,
      });
    default:
      throw new Error(`Unknown transport kind: ${kind}`);
  }
}
