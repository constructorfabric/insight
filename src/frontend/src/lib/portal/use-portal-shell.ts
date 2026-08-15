import { useRouterState } from "@tanstack/react-router";

import { isPortalShellPath } from "@/lib/portal/portal-routes";
import { usePortalEnabled } from "@/lib/portal/portal-store";

/**
 * Whether the portal owns the screen right now.
 *
 * The root layout answers this to pick a shell, and chrome shared by both
 * shells answers it to pick a link target. Two copies of the condition drift:
 * the shared sidebar footer kept naming `/metrics` and `/whats-new` from
 * inside the portal, which swapped the whole shell back to the previous
 * interface (constructorfabric/insight#2569).
 */
export function useInPortalShell(): boolean {
  const enabled = usePortalEnabled();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  return enabled && isPortalShellPath(pathname);
}
