import { useNavigate } from "@tanstack/react-router";
import { useCallback, useMemo } from "react";

import { normalizePersonId } from "@/lib/metrics/entity";
import { type OrgScope, usePortalShowPlanned } from "@/lib/portal/portal-store";
import { linkTo, type NavTarget } from "@/lib/portal/portal-routes";
import { usePortalSearch, useSetPortalSearch } from "@/lib/portal/portal-search";
import { useNavLocation } from "@/lib/portal/use-nav-location";
import { recordUsageEvent, scopeLabel } from "@/telemetry";

/** The active slice attribute, or "" while no cohort is in effect. */
export function usePortalSlice(): string {
  const slice = usePortalSearch().slice ?? "";
  return usePortalShowPlanned() ? slice : "";
}

/** The org scope the URL names: a root person id (absent = the viewer) + direct-only. */
export function usePortalScope(): OrgScope {
  const { scope, direct } = usePortalSearch();
  // Memoised on the primitives: `useOrgScope` lists this object in a dependency
  // array, and a fresh literal per render would re-walk the whole identity tree.
  return useMemo(
    () => ({ root: scope ?? null, directOnly: direct ?? false }),
    [scope, direct],
  );
}

/** Navigate to a portal page. One call is one history entry. */
export function useGo(): (target: NavTarget, opts?: { replace?: boolean }) => void {
  const navigate = useNavigate();
  return useCallback(
    (target, opts) => {
      void navigate({ ...linkTo(target), replace: opts?.replace ?? false });
    },
    [navigate],
  );
}

/**
 * Writers for the navigation state. `set*` pushes a history entry (a reader
 * moved); `replace*` corrects the URL without one (an effect did).
 */
export interface PortalNavActions {
  /** Open another item of the zone on screen. */
  setItem: (item: string | null) => void;
  openItem: (zone: "overview" | "aicost" | "reports" | "manage", item: string) => void;
  setAcct: (acct: string | null) => void;
  /** Open a direction on a lens, or close it (`""`). */
  openDirection: (dir: string, lens?: string) => void;
  /** Descend into one repository of the current lens, or leave it (""). */
  openRepository: (repo: string) => void;
  setSlice: (slice: string) => void;
  setScope: (patch: Partial<OrgScope>) => void;
  replaceScope: (patch: Partial<OrgScope>) => void;
}

/** Stable navigation writers — safe to list in an effect's dependencies. */
export function usePortalNavActions(): PortalNavActions {
  const setSearch = useSetPortalSearch();
  const go = useGo();
  const { zone, person } = useNavLocation();

  return useMemo(
    () => ({
      setItem: (item) => {
        const picked = item ?? undefined;
        if (zone === "person") go({ zone, person, section: picked });
        else if (zone === "people") go({ zone, person, item: picked });
        else if (zone === "overview" || zone === "aicost" || zone === "reports" || zone === "manage") {
          go({ zone, item: picked });
        }
      },
      openItem: (target, item) => go({ zone: target, item }),
      setAcct: (acct) => setSearch({ acct: acct ?? undefined }),
      openDirection: (dir, lens) => go({ zone: "directions", dir: dir || undefined, lens }),
      openRepository: (repo) => setSearch({ repo: repo || undefined }),
      setSlice: (slice) => {
        recordUsageEvent("cohort", slice || "none");
        setSearch({ slice: slice || undefined });
      },
      replaceScope: (patch) =>
        setSearch(
          { ...("root" in patch ? { scope: patch.root ?? undefined } : {}) },
          { replace: true },
        ),
      setScope: (patch) => {
        recordUsageEvent(
          "scope",
          scopeLabel({
            root: patch.root ?? null,
            directOnly: patch.directOnly ?? false,
            attrFilter: patch.attrFilter,
          }),
        );
        // Derived from the PREVIOUS search rather than a captured render value,
        // which is what keeps this callback stable.
        return setSearch((prev) => ({
          ...("root" in patch ? { scope: patch.root ?? undefined } : {}),
          ...("directOnly" in patch ? { direct: patch.directOnly } : {}),
          // A direct-only narrowing rarely survives a new root: reset it when
          // the root itself moves and the caller did not say otherwise.
          ...("root" in patch &&
          !("directOnly" in patch) &&
          normalizePersonId(patch.root ?? "") !== normalizePersonId(prev.scope ?? "")
            ? { direct: undefined }
            : {}),
        }));
      },
    }),
    [setSearch, go, zone, person],
  );
}
