// Layer 1 — HTTP API. Built on node:http only (no Express).
//
// Two routes:
//   GET /api/*    -> proxy to https://claude.ai/api/* via transport
//   GET /health   -> readiness probe
// Everything else: 404 / 405.
//
// Error model:
//   - transport.fetch threw  -> 502 "upstream unreachable"
//   - transport not ready    -> 503 "transport not ready"
//   - method != GET          -> 405
//   - path doesn't match     -> 404
//   - any other exception    -> 500 (last-resort safety net)

import http from 'node:http';

import { log } from './log.js';

/**
 * Create an http.Server bound to a transport. The server does not own
 * the transport lifecycle — index.js does init() + close() around it.
 *
 * @param {import('./transport/index.js').AuthedTransport} transport
 * @returns {import('node:http').Server}
 */
export function createServer(transport) {
  return http.createServer((req, res) => {
    handle(req, res, transport).catch((err) => {
      // Last-resort safety net. Should never reach here unless handle()
      // itself throws synchronously, which it shouldn't.
      log.error('handler unhandled', { error: err.message, stack: err.stack });
      if (!res.headersSent) {
        respond(res, 500, { error: 'internal server error' });
      }
    });
  });
}

/**
 * @param {import('node:http').IncomingMessage} req
 * @param {import('node:http').ServerResponse} res
 * @param {import('./transport/index.js').AuthedTransport} transport
 */
async function handle(req, res, transport) {
  if (req.method !== 'GET') {
    return respond(res, 405, { error: 'method not allowed', method: req.method });
  }

  // req.url is path+search (no host). new URL needs a base — we use a
  // dummy one because we only care about pathname/search of the
  // incoming request, not the host.
  const reqUrl = new URL(req.url ?? '/', 'http://internal');

  if (reqUrl.pathname === '/health') {
    return handleHealth(res, transport);
  }

  if (reqUrl.pathname.startsWith('/api/')) {
    return handleProxy(res, transport, reqUrl);
  }

  return respond(res, 404, { error: 'not found', path: reqUrl.pathname });
}

/**
 * Health endpoint — synchronous (no async I/O) so it's safe to hit at
 * high frequency from k8s probes.
 */
function handleHealth(res, transport) {
  const ready = transport.isReady();
  return respond(res, ready ? 200 : 503, {
    status: ready ? 'ok' : 'init',
    ready,
    transport: transport.kind,
  });
}

/**
 * Proxy /api/* to upstream. Pass through status+body verbatim.
 */
async function handleProxy(res, transport, reqUrl) {
  if (!transport.isReady()) {
    return respond(res, 503, {
      error: 'transport not ready',
      detail: 'init has not finished yet',
    });
  }

  // Map /api/X?Y -> {upstreamBaseUrl}/api/X?Y. The base URL lives on
  // the transport so the server never has to know any specific
  // upstream hostname — swap transport, server keeps working.
  const upstreamUrl = `${transport.upstreamBaseUrl}${reqUrl.pathname}${reqUrl.search}`;

  const t0 = Date.now();
  let result;
  try {
    result = await transport.fetch(upstreamUrl);
  } catch (err) {
    log.error('transport.fetch failed', {
      upstream: upstreamUrl,
      error: err.message,
    });
    // Distinguish timeout (504) from generic transport failure (502)
    // so callers can apply different retry/backoff policies.
    const isTimeout = err.message?.startsWith('fetch timeout');
    return respond(res, isTimeout ? 504 : 502, {
      error: isTimeout ? 'upstream timeout' : 'upstream unreachable',
      detail: err.message,
    });
  }
  const ms = Date.now() - t0;

  log.info('proxy', {
    upstream: upstreamUrl,
    status: result.status,
    size: result.body.length,
    ms,
  });

  // Pass through status + body. Set content-type from upstream if
  // available; default to application/json (claude.ai api is always
  // JSON-shaped).
  res.statusCode = result.status;
  res.setHeader(
    'content-type',
    result.headers['content-type'] ?? 'application/json',
  );
  // Debug-only header so an operator hitting the proxy can see which
  // transport served the response.
  res.setHeader('x-proxy-transport', transport.kind);
  res.end(result.body);
}

/**
 * Write a JSON response with the given status. Used for all server-
 * generated responses (not proxy passthrough).
 */
function respond(res, status, body) {
  res.statusCode = status;
  res.setHeader('content-type', 'application/json');
  res.end(JSON.stringify(body));
}
