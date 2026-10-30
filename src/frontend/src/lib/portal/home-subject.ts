import { OWN_OVERVIEW_ITEMS } from "@/lib/portal/overview-configs";
import { useViewerReach } from "@/lib/portal/use-viewer-reach";
import {
  usePortalSearch,
  useSetPortalSearch,
} from "@/lib/portal/portal-search";

/** Whom Home and its themes are about: the viewer alone, or their team. */
export type HomeSubject = "me" | "team";

export function homeSubject(
  canSeeOthers: boolean,
  who: "me" | undefined
): HomeSubject {
  return canSeeOthers && who !== "me" ? "team" : "me";
}

export function homeItemShown(subject: HomeSubject, item: string): boolean {
  return subject === "team" || item in OWN_OVERVIEW_ITEMS;
}

export function useHomeSubject(): {
  subject: HomeSubject;
  canSwitch: boolean;
  isPending: boolean;
  setSubject: (next: HomeSubject) => void;
} {
  const { canSeeOthers, isPending, isError } = useViewerReach();
  const { who } = usePortalSearch();
  const setSearch = useSetPortalSearch();

  return {
    subject: homeSubject(canSeeOthers, who),
    canSwitch: canSeeOthers,
    // A failed roster leaves the viewer's own views, which need no roster.
    isPending: isPending && !isError,
    setSubject: (next) => setSearch({ who: next === "me" ? "me" : undefined }),
  };
}
