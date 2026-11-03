import { OWN_OVERVIEW_ITEMS } from "@/lib/portal/overview-configs";
import { useViewerReach } from "@/lib/portal/use-viewer-reach";

/** Whom Home and its themes are about: the viewer alone, or their team. */
export type HomeSubject = "me" | "team";

export function homeSubject(canSeeOthers: boolean): HomeSubject {
  return canSeeOthers ? "team" : "me";
}

export function homeItemShown(subject: HomeSubject, item: string): boolean {
  return subject === "team" || item in OWN_OVERVIEW_ITEMS;
}

export function useHomeSubject(): {
  subject: HomeSubject;
  isPending: boolean;
} {
  const { canSeeOthers, isPending, isError } = useViewerReach();

  return {
    subject: homeSubject(canSeeOthers),
    // A failed roster leaves the viewer's own views, which need no roster.
    isPending: isPending && !isError,
  };
}
