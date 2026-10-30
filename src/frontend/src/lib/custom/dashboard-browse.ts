import { useInfiniteQuery } from "@tanstack/react-query";
import { LayoutGrid, Share2, User, type LucideIcon } from "lucide-react";

import type { OwnedFilter } from "@/api/custom-client";
import { definitionPagesQuery } from "@/queries/custom";
import { useIsAdmin } from "@/queries/identity-me";

export interface BrowseEntry {
  owned: OwnedFilter | undefined;
  label: string;
  icon: LucideIcon;
  adminOnly: boolean;
}

const BROWSE: readonly BrowseEntry[] = [
  { owned: undefined, label: "All dashboards", icon: LayoutGrid, adminOnly: false },
  { owned: "me", label: "My dashboards", icon: User, adminOnly: true },
  { owned: "shared", label: "Shared with me", icon: Share2, adminOnly: false },
];

export interface BrowseRow extends BrowseEntry {
  count: number | undefined;
}

// INVARIANT: the pane and the chips above the list both read this, so they
// always offer the same choices with the same counts.
export function useDashboardBrowse(tags: string[] = []): BrowseRow[] {
  const { isAdmin } = useIsAdmin();
  const all = useInfiniteQuery(definitionPagesQuery("dashboards", "", { tags }));
  const mine = useInfiniteQuery({
    ...definitionPagesQuery("dashboards", "", { owned: "me", tags }),
    enabled: isAdmin,
  });
  const shared = useInfiniteQuery(
    definitionPagesQuery("dashboards", "", { owned: "shared", tags })
  );
  const counts = new Map<OwnedFilter | undefined, number | undefined>([
    [undefined, all.data?.pages.at(-1)?.total],
    ["me", mine.data?.pages.at(-1)?.total],
    ["shared", shared.data?.pages.at(-1)?.total],
  ]);

  return BROWSE.filter((entry) => isAdmin || !entry.adminOnly).map(
    (entry) => ({ ...entry, count: counts.get(entry.owned) })
  );
}
