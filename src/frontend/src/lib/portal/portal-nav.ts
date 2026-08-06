import { useRouterState } from "@tanstack/react-router";
import { useMemo } from "react";

import { normalizePersonId } from "@/lib/metrics/entity";
import type { OrgScope } from "@/lib/portal/portal-store";
import { usePortalSearch, useSetPortalSearch } from "@/lib/portal/portal-search";

/**
 * Portal navigation, read from and written to the URL.
 *
 * These replace the in-memory store the shell used to keep: reloading no
 * longer resets the view, a link reproduces it, and every zone/lens change is
 * a history entry, so Back goes where a reader expects. The one asymmetry is
 * deliberate — Person and People come from the PATH (`/ic/<email>/personal`,
 * `/ic/<email>/team`) because they are about a person, while theme zones ride
 * in `?zone=`. A path-driven zone therefore always wins over a stale param.
 */

/** Zone from the route when the path names one, else from `?zone=`. */
export function usePortalZone(): string | null {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { zone } = usePortalSearch();
  if (/^\/ic\/[^/]+\/team\/?$/.test(pathname)) return "people";
  if (/^\/ic\/[^/]+\/personal\/?$/.test(pathname)) return "person";
  return zone ?? null;
}

/** The selected item within a zone; null when the zone shows its default. */
export function usePortalItem(): string | null {
  return usePortalSearch().item ?? null;
}

/** The expanded direction, or "" when none is open. */
export function usePortalDir(): string {
  return usePortalSearch().dir ?? "";
}

/** The active lens inside the open direction, or "" before one is picked. */
export function usePortalLens(): string {
  return usePortalSearch().lens ?? "";
}

/** The active slice attribute, or "" when the roster is one undivided cohort. */
export function usePortalSlice(): string {
  return usePortalSearch().slice ?? "";
}

/** The org scope the URL names: a root person id (absent = the viewer) + direct-only. */
export function usePortalScope(): OrgScope {
  const { scope, direct } = usePortalSearch();
  // Memoised on the primitives: `useOrgScope` lists this object in a dependency
  // array, and a fresh literal per render would re-walk the whole identity tree
  // (flattenSubordinates per manager node) on every render of every org zone.
  return useMemo(
    () => ({ root: scope ?? null, directOnly: direct ?? false }),
    [scope, direct],
  );
}

/**
 * Writers for the navigation state. `set*` pushes a history entry (a reader
 * moved); `replace*` corrects the URL without one (an effect did).
 */
export interface PortalNavActions {
  /** Correct the URL without adding a history entry (effects, not clicks). */
  replaceZone: (zone: string | null) => void;
  replaceScope: (patch: Partial<OrgScope>) => void;
  setZone: (zone: string | null) => void;
  setItem: (item: string | null) => void;
  setDir: (dir: string) => void;
  setLens: (lens: string) => void;
  setSlice: (slice: string) => void;
  setScope: (patch: Partial<OrgScope>) => void;
}

/** Stable navigation writers — safe to list in an effect's dependencies. */
export function usePortalNavActions(): PortalNavActions {
  const setSearch = useSetPortalSearch();
  // Memoised so callers can list these in effect dependencies: a fresh object
  // per render would re-run the landing-zone and scope-sync effects forever.
  return useMemo(
    () => ({
      // A zone change drops the item with it: `item` is per-zone, and carrying
      // it across renders a fallback view while the pane highlights nothing.
      setZone: (zone) => setSearch({ zone: zone ?? undefined, item: undefined }),
      replaceZone: (zone) =>
        setSearch({ zone: zone ?? undefined, item: undefined }, { replace: true }),
      replaceScope: (patch) =>
        setSearch(
          { ...("root" in patch ? { scope: patch.root ?? undefined } : {}) },
          { replace: true },
        ),
      setItem: (item) => setSearch({ item: item ?? undefined }),
      setDir: (dir) => setSearch({ dir: dir || undefined }),
      setLens: (lens) => setSearch({ lens: lens || undefined }),
      setSlice: (slice) => setSearch({ slice: slice || undefined }),
      setScope: (patch) =>
        // Derived from the PREVIOUS search rather than a captured render value,
        // which is what keeps this callback stable.
        setSearch((prev) => ({
          ...("root" in patch ? { scope: patch.root ?? undefined } : {}),
          ...("directOnly" in patch ? { direct: patch.directOnly } : {}),
          // A direct-only narrowing rarely survives a new root: reset it when
          // the root itself moves and the caller did not say otherwise.
          ...("root" in patch &&
          !("directOnly" in patch) &&
          normalizePersonId(patch.root ?? "") !==
            normalizePersonId(prev.scope ?? "")
            ? { direct: undefined }
            : {}),
        })),
    }),
    [setSearch],
  );
}
