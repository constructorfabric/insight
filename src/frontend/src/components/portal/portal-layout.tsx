import { useEffect, useRef, type CSSProperties, type ReactNode } from "react";
import { Navigate, Outlet } from "@tanstack/react-router";

import { MockBanner } from "@/components/mock-banner";
import { ViewAsBanner } from "@/components/view-as-banner";
import { ContextPane } from "@/components/portal/context-pane";
import { LensRail } from "@/components/portal/lens-rail";
import { PortalTopBar } from "@/components/portal/portal-topbar";
import {
  SidebarInset,
  SidebarProvider,
  useSidebar,
} from "@/components/ui/sidebar";
import { bareLandingZone, landingDecision } from "@/lib/portal/landing-zone";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import {
  useShellLayout,
  type ShellLayout,
} from "@/lib/portal/use-shell-layout";
import { CONTENT_ID } from "@/lib/portal/content-scroll";
import { useNavLocation } from "@/lib/portal/use-nav-location";
import { useViewerReach } from "@/lib/portal/use-viewer-reach";
import { useIsAdmin } from "@/queries/identity-me";

/**
 * Portal shell (Phase 1 buildout), rendered unless the reader opts out via
 * `insight.portal`. Composition (one SidebarProvider, all normal flow):
 *   [ lens rail ] [ zone-contextual pane ] [ content ]
 * Mounted once by the `_shell` route: the outlet swaps the page, never the shell.
 */
export function PortalLayout() {
  return (
    <SidebarProvider
      className="h-svh overflow-hidden"
      style={{ "--rail-width": "3.5rem" } as CSSProperties}
    >
      <PaneStateForLayout />
      <LensRail />
      <ContextPane />
      {/* INVARIANT: `isolate` keeps the inset's own stacking — the sticky
          topbar included — under the rail and the pane, which open across it
          on the tier where the pane is off-canvas. */}
      <SidebarInset
        id={CONTENT_ID}
        data-scroll-restoration-id={CONTENT_ID}
        className="isolate min-w-0 overflow-x-clip overflow-y-auto"
      >
        <MockBanner />
        {/* The impersonation indicator: it names whose data is on screen and
            carries the way out. Missing it left a view-as operator with no sign
            they were not looking at their own org, and no exit. */}
        <ViewAsBanner />
        <PortalTopBar />
        <ZoneGuard>
          <Outlet />
        </ZoneGuard>
      </SidebarInset>
    </SidebarProvider>
  );
}

/**
 * The pane is in normal flow only on a wide screen; narrower, it is off-canvas
 * and must START collapsed, or a tablet is back to 312px of chrome. The
 * provider's `open` defaults to true and survives a resize, so a layout change
 * has to reset it.
 *
 * Guarded on the layout actually CHANGING: `setOpen` from the provider is a new
 * function on every open-state change, so an unguarded effect would re-fire and
 * slam the pane shut the instant the reader opened it.
 */
function PaneStateForLayout() {
  const layout = useShellLayout();
  const { setOpen } = useSidebar();
  const previous = useRef<ShellLayout | null>(null);
  useEffect(() => {
    if (previous.current === layout) return;
    previous.current = layout;
    setOpen(layout === "wide");
  }, [layout, setOpen]);
  return null;
}

/**
 * Send the reader home from a zone that is not theirs to see. The rules live
 * in `landingDecision` (pure, table-tested); the zone renders while the
 * viewer's reach is still resolving.
 */
function ZoneGuard({ children }: { children: ReactNode }) {
  const { zone } = useNavLocation();
  const { canSeeOthers, isPending } = useViewerReach();
  const { isAdmin, isPending: adminPending, isError: adminError } = useIsAdmin();
  const homeShown = bareLandingZone(usePortalShowPlanned()) === "overview";
  const decision = landingDecision({
    zone,
    mgrPending: isPending,
    canSeeOthers,
    // An errored check is "unknown", not "no": refusing on it would bounce an
    // admin off a page they opened. The me query retries until an answer lands.
    adminPending: adminPending || adminError,
    isAdmin,
    homeShown,
  });

  if (decision.kind === "reset") return <Navigate to="/" replace />;
  return children;
}
