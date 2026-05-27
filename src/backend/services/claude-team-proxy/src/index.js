// Entry point. Orchestrates the lifecycle:
//   1. Load config.
//   2. Create transport (cheap, no I/O).
//   3. Start HTTP server (so /health responds immediately).
//   4. transport.init() (slow: Chromium launch + CF challenge).
//   5. Serve traffic.
//   6. On SIGTERM/SIGINT: stop accepting, close transport, exit.
//
// The order in 3-4 matters for k8s: readiness probe must not 404 while
// init is in progress. /health returns 503 "init" until step 4 finishes.

import { loadConfig } from './config.js';
import { createTransport } from './transport/index.js';
import { createServer } from './server.js';
import { log } from './log.js';

const SHUTDOWN_GRACE_MS = 30_000;

async function main() {
  // 1) Config — throws on bad/missing input, process exits with code 2.
  let config;
  try {
    config = loadConfig();
  } catch (err) {
    log.error('config load failed', { error: err.message });
    process.exit(2);
  }
  log.info('config loaded', {
    port: config.port,
    upstreamBaseUrl: config.upstreamBaseUrl,
    headless: config.headless,
    startupTimeoutMs: config.startupTimeoutMs,
    fetchTimeoutMs: config.fetchTimeoutMs,
  });

  // 2) Create transport object — no I/O happens here.
  const transport = createTransport({
    sessionKey: config.sessionKey,
    upstreamBaseUrl: config.upstreamBaseUrl,
    headless: config.headless,
    startupTimeoutMs: config.startupTimeoutMs,
    fetchTimeoutMs: config.fetchTimeoutMs,
  });

  // 3) Start HTTP server BEFORE transport.init(). Critical for k8s:
  //    readiness probe needs the socket open immediately. /health
  //    returns 503 until transport is ready.
  const server = createServer(transport);
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(config.port, () => {
      log.info('http server listening', { port: config.port });
      resolve();
    });
  });

  // 4) Wire signal handlers BEFORE init(). If init() hangs and we get
  //    SIGTERM mid-init, we still need to clean up.
  installShutdownHandlers(server, transport);

  // 5) Init transport. May take tens of seconds (CF challenge).
  try {
    await transport.init();
  } catch (err) {
    log.error('transport.init failed, exiting', { error: err.message });
    // Best-effort cleanup. server.close is async but we're exiting
    // anyway; not waiting on it.
    server.close();
    await transport.close().catch(() => {});
    process.exit(1);
  }

  log.info('proxy ready');
  // From here we just serve requests until a signal arrives. No
  // explicit loop; node:http keeps the event loop alive.
}

/**
 * @param {import('node:http').Server} server
 * @param {import('./transport/index.js').AuthedTransport} transport
 */
function installShutdownHandlers(server, transport) {
  let shuttingDown = false;

  const handler = async (signal) => {
    if (shuttingDown) {
      log.warn('signal received during shutdown, ignoring', { signal });
      return;
    }
    shuttingDown = true;
    log.info('shutdown initiated', { signal });

    // Hard timeout — if cleanup takes too long, force-exit. Otherwise
    // a stuck transport.close() would leave the pod in Terminating
    // state until k8s SIGKILLs it (defaults to 30s anyway).
    const forceExit = setTimeout(() => {
      log.error('shutdown timeout, force-exit');
      process.exit(1);
    }, SHUTDOWN_GRACE_MS);
    forceExit.unref(); // don't keep event loop alive just for this

    // Stop accepting new connections. Existing in-flight requests
    // continue until they finish (or the grace timeout fires).
    server.close((err) => {
      if (err) log.error('server.close error', { error: err.message });
    });

    try {
      await transport.close();
    } catch (err) {
      log.error('transport.close error', { error: err.message });
    }

    clearTimeout(forceExit);
    log.info('shutdown complete');
    process.exit(0);
  };

  process.on('SIGTERM', () => void handler('SIGTERM'));
  process.on('SIGINT', () => void handler('SIGINT'));

  // Process-wide error nets. Should never fire in a healthy run —
  // log them loudly so we notice in monitoring.
  process.on('unhandledRejection', (reason) => {
    log.error('unhandled promise rejection', {
      reason: reason instanceof Error ? reason.message : String(reason),
    });
  });
  process.on('uncaughtException', (err) => {
    log.error('uncaught exception', {
      error: err.message,
      stack: err.stack,
    });
    process.exit(1);
  });
}

main().catch((err) => {
  log.error('main crashed', { error: err.message, stack: err.stack });
  process.exit(1);
});
