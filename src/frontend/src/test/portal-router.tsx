import { useEffect, useSyncExternalStore } from "react";
import { vi } from "vitest";

import type { PortalSearch } from "@/lib/portal/portal-search";

/**
 * Portal navigation lives in the URL, so a component test needs a URL rather
 * than a store to say "the reader is on Directions, sliced by division".
 *
 * A full memory router would drag the whole route tree into every component
 * test; this stubs the router hooks the portal actually reads and records the
 * navigations a component issues — which IS the behaviour now, not an
 * implementation detail worth hiding.
 *
 * Usage (the async factory is what lets a shared helper back a `vi.mock`,
 * since mock factories are hoisted above the file's own imports):
 *
 * ```ts
 * vi.mock("@tanstack/react-router", async () => {
 *   const { portalRouterMock } = await import("@/test/portal-router");
 *   return portalRouterMock();
 * });
 * import { portalRouter } from "@/test/portal-router";
 * ```
 *
 * The state is a per-file singleton — vitest gives each test file its own
 * module registry — so `portalRouter.reset()` in `beforeEach` is enough.
 */
const listeners = new Set<() => void>();
function emit(): void {
  for (const fn of listeners) fn();
}

export const portalRouter = {
  search: {} as PortalSearch,
  pathname: "/portal",
  /** Every `navigate()` a component issued, newest last. */
  navigations: [] as Array<Record<string, unknown>>,

  /** Patch the query string, as a link or a control would. */
  set(next: Partial<PortalSearch>): void {
    const merged: Record<string, unknown> = { ...this.search, ...next };
    for (const [k, v] of Object.entries(next)) {
      if (v === undefined) delete merged[k];
    }
    this.search = merged as PortalSearch;
    emit();
  },

  /** Move to a path, as a route change would. */
  go(pathname: string): void {
    this.pathname = pathname;
    emit();
  },

  reset(pathname = "/portal"): void {
    this.search = {};
    this.pathname = pathname;
    this.navigations = [];
    emit();
  },
};

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/**
 * The router module replacement: subscribed hooks, a `Link` that emits a real
 * href, and a `Navigate` that records the redirect a guard fires.
 */
export function portalRouterMock(): Record<string, unknown> {
  const navigate = vi.fn((opts: Record<string, unknown>) => {
    portalRouter.navigations.push(opts);
    // Routed through `set` so a cleared key is DELETED rather than left as
    // `undefined`: the real router drops it from the URL, and a fake that keeps
    // it makes `toEqual` assertions disagree with what a shared link contains.
    if (typeof opts.search === "function") {
      const next = (opts.search as (p: unknown) => Record<string, unknown>)(
        portalRouter.search,
      );
      portalRouter.search = {} as PortalSearch;
      portalRouter.set(next as Partial<PortalSearch>);
    } else if (opts.search) {
      portalRouter.search = {} as PortalSearch;
      portalRouter.set(opts.search as Partial<PortalSearch>);
    }
    if (typeof opts.to === "string" && opts.to !== ".") {
      const params = (opts.params ?? {}) as Record<string, string>;
      portalRouter.go(
        opts.to.replace(/\$(\w+)/g, (_, k: string) =>
          encodeURIComponent(params[k] ?? ""),
        ),
      );
    }
  });
  return {
    useNavigate: () => navigate,
    // Subscribed, not snapshot-read: a component must re-render when the URL
    // changes, which is the behaviour the portal now depends on entirely.
    useSearch: () =>
      useSyncExternalStore(
        subscribe,
        () => portalRouter.search,
        () => portalRouter.search,
      ),
    useRouterState: ({ select }: { select: (s: unknown) => unknown }) => {
      const path = useSyncExternalStore(
        subscribe,
        () => portalRouter.pathname,
        () => portalRouter.pathname,
      );
      const search = useSyncExternalStore(
        subscribe,
        () => portalRouter.search,
        () => portalRouter.search,
      );
      return select({ location: { pathname: path, search } });
    },
    // A guard's redirect is a navigation too — recorded so a test can assert
    // that a disabled preview or an invalid param sends the reader away.
    Navigate: ({ to, replace }: { to: string; replace?: boolean }) => {
      useEffect(() => {
        portalRouter.navigations.push({ to, replace: replace ?? false });
      }, [to, replace]);
      return null;
    },
    // A real href, not a bare <a>: without it there is no link role to query,
    // and the point of this migration is that a link CARRIES the state — so a
    // test can assert what a shared URL would contain.
    Link: ({
      to,
      params,
      search,
      children,
      ...rest
    }: {
      to?: string;
      params?: Record<string, string>;
      search?: Record<string, unknown> | ((prev: unknown) => Record<string, unknown>);
      children: React.ReactNode;
    } & Record<string, unknown>) => {
      const path = (to ?? "").replace(/\$(\w+)/g, (_, k: string) =>
        encodeURIComponent(params?.[k] ?? ""),
      );
      const resolved =
        typeof search === "function" ? search(portalRouter.search) : search;
      const qs = new URLSearchParams(
        Object.entries(resolved ?? {})
          .filter(([, v]) => v !== undefined && v !== "" && v !== false)
          .map(([k, v]) => [k, String(v)]),
      ).toString();
      return (
        <a href={qs ? `${path}?${qs}` : path} {...rest}>
          {children}
        </a>
      );
    },
  };
}
