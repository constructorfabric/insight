import type { ClientRequest, IncomingMessage } from "node:http";
import type { Plugin, ProxyOptions } from "vite";

// Without an upstream, vite's SPA fallback serves /auth/login itself and the
// login redirect loops — so dev always proxies somewhere.
const DEFAULT_TARGET = "http://localhost:8080";

const SESSION_COOKIE = "__Host-sid";
const SESSION_PAIR = new RegExp(`(?:^|;\\s*)${SESSION_COOKIE}=([^;]+)`);
const PROXIED_PREFIXES = ["/api", "/auth"];

/**
 * The session the dev server holds on the developer's behalf when
 * `VITE_API_PROXY_SESSION` points `pnpm dev` at a deployed stand.
 *
 * A stand registers its OIDC `redirect_uri` on its own origin, so the login leg
 * can never complete against localhost: the IdP posts the code back to the
 * stand, which mints the session there and leaves the dev server with nothing.
 * Injecting a session the developer already holds is what keeps the loop
 * same-origin. The gateway reads `__Host-sid` straight off the Cookie header
 * and the CSRF check clears on the `X-CSRF-Token` the SPA reads from
 * `/auth/me`, so neither cares that the request arrived through a proxy.
 *
 * The token lives here rather than in the browser: `/auth/refresh` rotates it
 * every few minutes, and the response cookie is `Secure` + `__Host-`, which a
 * dev server reached over plain http on anything but localhost cannot store.
 */
type DevSession = {
  /**
   * Fixed at construction, not derived from the current token — the session
   * dropping (logout, expiry) must leave the login guard in place rather than
   * silently reverting to a login that abandons the dev server.
   */
  readonly enabled: boolean;
  cookieHeader(incoming: string | number | string[] | undefined): string;
  absorbSetCookie(setCookie: string[] | undefined): string[] | undefined;
};

function splitCookie(incoming: string | number | string[] | undefined): string[] {
  const raw = Array.isArray(incoming) ? incoming.join("; ") : String(incoming ?? "");
  return raw
    .split(";")
    .map((pair) => pair.trim())
    .filter(Boolean);
}

function createDevSession(seed: string): DevSession {
  let token = seed;

  return {
    enabled: seed !== "",

    cookieHeader(incoming) {
      const pairs = splitCookie(incoming).filter(
        (pair) => !pair.startsWith(`${SESSION_COOKIE}=`)
      );
      if (token) pairs.push(`${SESSION_COOKIE}=${token}`);
      return pairs.join("; ");
    },

    absorbSetCookie(setCookie) {
      if (!setCookie) return undefined;
      const kept: string[] = [];
      for (const directive of setCookie) {
        if (!directive.startsWith(`${SESSION_COOKIE}=`)) {
          kept.push(directive);
          continue;
        }
        // A rotation carries the next token; logout's clear-cookie is empty,
        // which drops the session and trips the login guard on the next 401.
        token = directive.slice(SESSION_COOKIE.length + 1).split(";")[0] ?? "";
      }
      return kept.length > 0 ? kept : undefined;
    },
  };
}

/** Accepts a bare token, a `__Host-sid=…` pair, or a whole Cookie header. */
export function normalizeSessionToken(raw: string): string {
  const trimmed = raw.trim().replace(/^["']|["']$/g, "");
  if (!trimmed) return "";
  return (SESSION_PAIR.exec(trimmed)?.[1] ?? trimmed.split(";")[0] ?? "").trim();
}

function proxyEntry(
  target: string,
  session: DevSession,
  secure: boolean
): ProxyOptions {
  const targetOrigin = new URL(target).origin;

  return {
    target,
    changeOrigin: true,
    secure,
    configure(proxy) {
      if (!session.enabled) return;

      proxy.on("proxyReq", (proxyReq: ClientRequest) => {
        const cookie = session.cookieHeader(proxyReq.getHeader("cookie"));
        if (cookie) proxyReq.setHeader("cookie", cookie);
        else proxyReq.removeHeader("cookie");
      });

      proxy.on("proxyRes", (proxyRes: IncomingMessage) => {
        const kept = session.absorbSetCookie(proxyRes.headers["set-cookie"]);
        if (kept) proxyRes.headers["set-cookie"] = kept;
        else delete proxyRes.headers["set-cookie"];

        // changeOrigin makes the stand emit absolute Locations on its own
        // public host; left alone they navigate the browser off the dev server.
        const location = proxyRes.headers.location;
        if (typeof location === "string" && location.startsWith(targetOrigin)) {
          proxyRes.headers.location = location.slice(targetOrigin.length) || "/";
        }
      });
    },
  };
}

function loginGuidance(target: string): string {
  return [
    "Insight dev proxy blocked /auth/login.",
    "",
    `${target} registers its OIDC redirect_uri on its own origin, so a login`,
    "started here finishes there and this dev server never gets a session.",
    "",
    `The SPA asked to log in because ${target} rejected`,
    "VITE_API_PROXY_SESSION — expired, revoked, or mistyped.",
    "",
    `Sign in to ${target} in a browser, copy the ${SESSION_COOKIE} value from`,
    "DevTools > Application > Cookies, set VITE_API_PROXY_SESSION in",
    "src/frontend/.env.local, and restart `pnpm dev`.",
  ].join("\n");
}

/**
 * Proxy `/api` and `/auth` to the dev backend, optionally carrying a session
 * for a deployed one. Serve-only: a build has no upstream to talk to.
 */
export function backendProxy(env: Record<string, string>): Plugin {
  const target = env.VITE_API_PROXY_TARGET || DEFAULT_TARGET;
  const secure = env.VITE_API_PROXY_INSECURE !== "true";
  const session = createDevSession(
    normalizeSessionToken(env.VITE_API_PROXY_SESSION ?? "")
  );

  return {
    name: "insight:backend-proxy",
    apply: "serve",

    config: () => ({
      server: {
        proxy: Object.fromEntries(
          PROXIED_PREFIXES.map((prefix) => [
            prefix,
            proxyEntry(target, session, secure),
          ])
        ),
      },
    }),

    configureServer(server) {
      if (!session.enabled) return;

      server.config.logger.warn(
        `[backend-proxy] attaching a ${SESSION_COOKIE} session to every ` +
          `${PROXIED_PREFIXES.join(" and ")} request to ${target}. That is a ` +
          "live session on that stand — keep VITE_API_PROXY_SESSION out of git."
      );

      // Registered from the hook body, so it runs ahead of vite's own proxy
      // middleware and answers instead of forwarding.
      server.middlewares.use((req, res, next) => {
        if (!req.url?.startsWith("/auth/login")) {
          next();
          return;
        }
        res.statusCode = 503;
        res.setHeader("content-type", "text/plain; charset=utf-8");
        res.end(loginGuidance(target));
      });
    },
  };
}
