import { useMemo } from "react";

import type { Gear } from "@/api/gear-roadmap-client";
import { UNGROUPED } from "@/lib/gears/roadmap-grid";
import { usePortalSubsystem } from "@/lib/portal/portal-nav";

/**
 * The gears a board shows: every one, or the subsystem the URL names. The
 * choice is in the URL rather than in each board's own state, so switching
 * lens keeps the narrowing — and so a narrowed board is a link.
 */
export function useSubsystemGears(gears: Gear[]): Gear[] {
  const chosen = usePortalSubsystem();

  return useMemo(() => {
    if (chosen === "") return gears;

    return gears.filter((gear) => (gear.subsystem ?? UNGROUPED) === chosen);
  }, [gears, chosen]);
}
