// PlaywrightTransport — AuthedTransport implementation using headless
// Chromium + puppeteer-extra-plugin-stealth.
//
// Adapted from phase0-fetch-members.js. Shape:
//   1. init() launches Chromium, injects sessionKey, navigates to
//      claude.ai, waits for Cloudflare clearance.
//   2. fetch() runs an in-page `fetch()` via page.evaluate(). The HTTP
//      request goes from Chromium (not Node) so it carries the real
//      Chromium TLS fingerprint, CF clearance cookies, and any other
//      site state.
//   3. close() shuts down the browser cleanly.
//
// Failure handling: if the browser process dies between fetch() calls,
// the next fetch() will throw and we drop `ready=false`. The supervisor
// (k8s) restarts the pod, init() re-runs from scratch.

import { chromium } from 'playwright-extra';
import StealthPlugin from 'puppeteer-extra-plugin-stealth';

import { log } from '../log.js';

chromium.use(StealthPlugin());

/**
 * @param {{
 *   sessionKey: string,
 *   upstreamBaseUrl: string,
 *   headless: boolean,
 *   startupTimeoutMs: number,
 *   fetchTimeoutMs: number,
 * }} cfg
 * @returns {import('./index.js').AuthedTransport}
 */
export function createPlaywrightTransport(cfg) {
  // Derive cookie domain from upstream URL so a mock upstream (e.g.
  // http://localhost:8080) can also receive the cookie. Leading dot
  // is the standard form for whole-domain match on hosts that have
  // a dot in the name; for bare hostnames like "localhost" we keep
  // the raw value.
  const upstreamHost = new URL(cfg.upstreamBaseUrl).hostname;
  const cookieDomain = upstreamHost.includes('.') ? `.${upstreamHost}` : upstreamHost;

  // Closure over private state. The returned object is the contract;
  // browser/context/page are intentionally not exposed.
  let browser = null;
  let context = null;
  let page = null;
  let ready = false;

  return {
    kind: 'playwright',
    upstreamBaseUrl: cfg.upstreamBaseUrl,

    isReady() {
      return ready;
    },

    async init() {
      if (ready) {
        log.info('transport.init called when already ready, no-op');
        return;
      }

      log.info('transport.init starting', {
        headless: cfg.headless,
        upstreamBaseUrl: cfg.upstreamBaseUrl,
      });
      const t0 = Date.now();

      try {
        browser = await chromium.launch({
          headless: cfg.headless,
          args: ['--disable-blink-features=AutomationControlled'],
        });

        context = await browser.newContext();
        await context.addCookies([{
          name: 'sessionKey',
          value: cfg.sessionKey,
          domain: cookieDomain,
          path: '/',
          secure: true,
          httpOnly: true,
          sameSite: 'Lax',
        }]);

        page = await context.newPage();

        log.info('navigating to upstream, may trigger CF challenge', {
          url: cfg.upstreamBaseUrl,
        });
        await page.goto(cfg.upstreamBaseUrl, {
          waitUntil: 'load',
          timeout: cfg.startupTimeoutMs,
        });

        // CF presents a "Just a moment..." interstitial. Stealth-patched
        // Chromium executes the JS challenge, CF then redirects to the
        // real page. We poll the document title every ~30ms via
        // waitForFunction.
        await page.waitForFunction(
          () => !document.title.includes('Just a moment'),
          { timeout: cfg.startupTimeoutMs },
        );

        ready = true;
        log.info('transport ready', {
          ms: Date.now() - t0,
          page_title: await page.title(),
        });
      } catch (err) {
        log.error('transport.init failed', {
          ms: Date.now() - t0,
          error: err.message,
        });
        // Best-effort cleanup; the outer process is likely going to
        // exit anyway, but don't leak chromium if not.
        await safeClose();
        throw err;
      }
    },

    async fetch(url, opts = {}) {
      if (!ready) {
        // Caller (server.js) checks isReady() before calling, so this
        // is defensive: should never happen via the HTTP path.
        throw new Error('transport not ready');
      }

      const method = opts.method ?? 'GET';
      if (method !== 'GET') {
        throw new Error(`PlaywrightTransport MVP supports GET only, got: ${method}`);
      }

      // page.evaluate serialises the callback + args, ships to the
      // browser, executes fetch() inside the page context (which has
      // sessionKey + CF clearance cookies).
      //
      // Return shape constrained to {status, body, headers} so it
      // matches the AuthedTransport contract; we don't bubble up the
      // full Response object.
      //
      // Timeout: the fetch is wrapped in AbortController so a stalled
      // upstream (Cloudflare hang, Anthropic outage) does not leave
      // the request indefinitely in-flight. On timeout the in-page
      // code returns `{timeout: true}`; we translate that to a
      // throw on the Node side per the AuthedTransport contract
      // (transport-level failures throw; non-2xx HTTP does not).
      const result = await page.evaluate(
        async ({ fetchUrl, extraHeaders, timeoutMs }) => {
          const controller = new AbortController();
          const timer = setTimeout(() => controller.abort(), timeoutMs);
          try {
            const response = await fetch(fetchUrl, {
              credentials: 'include',
              signal: controller.signal,
              headers: {
                'Cache-Control': 'no-cache, no-store',
                ...extraHeaders,
              },
            });
            // Headers.entries() is iterable; spread into a plain object
            // for transport boundary (page.evaluate output must be JSON-
            // serialisable).
            const headers = {};
            for (const [k, v] of response.headers.entries()) {
              headers[k.toLowerCase()] = v;
            }
            return {
              status: response.status,
              body: await response.text(),
              headers,
            };
          } catch (err) {
            // AbortError on timeout → signal to Node-side via timeout flag.
            // Any other Error (network failure inside the page, DNS, etc.)
            // bubbles a structured error envelope back; Node side
            // translates to a transport-level throw.
            if (err.name === 'AbortError') {
              return { timeout: true };
            }
            return { error: err.message || String(err) };
          } finally {
            clearTimeout(timer);
          }
        },
        { fetchUrl: url, extraHeaders: opts.headers ?? {}, timeoutMs: cfg.fetchTimeoutMs },
      );

      if (result.timeout) {
        throw new Error(`fetch timeout after ${cfg.fetchTimeoutMs}ms: ${url}`);
      }
      if (result.error) {
        throw new Error(`fetch failed: ${result.error}`);
      }
      return result;
    },

    async close() {
      await safeClose();
      ready = false;
    },
  };

  // ---- helpers (closed over the same vars) ----

  async function safeClose() {
    // Order matters: page → context → browser. Closing browser first
    // would leave context/page dangling and Playwright complains.
    if (page) {
      await page.close().catch(() => {});
      page = null;
    }
    if (context) {
      await context.close().catch(() => {});
      context = null;
    }
    if (browser) {
      await browser.close().catch(() => {});
      browser = null;
    }
  }
}
