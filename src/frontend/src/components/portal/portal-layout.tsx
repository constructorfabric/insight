import { useEffect, useRef } from "react";

import { MockBanner } from "@/components/mock-banner";
import { ViewAsBanner } from "@/components/view-as-banner";
import { ContextPane } from "@/components/portal/context-pane";
import { LensRail } from "@/components/portal/lens-rail";
import { PortalTopBar } from "@/components/portal/portal-topbar";
import { ZoneContent } from "@/components/portal/zone-content";
import { SidebarInset, SidebarProvider, useSidebar } from "@/components/ui/sidebar";
import {
  usePortalNavActions,
  usePortalZone,
} from "@/lib/portal/portal-nav";
import { useShellLayout, type ShellLayout } from "@/lib/portal/use-shell-layout";
import { useViewerIsManager } from "@/lib/portal/use-viewer-is-manager";

/**
 * Portal shell (Phase 1 buildout), behind the `insight.portal` flag so the
 * default app is untouched. Composition (one SidebarProvider, all normal flow):
 *   [ lens rail ] [ zone-contextual pane ] [ content ]
 * Every zone renders through `<ZoneContent/>` (Person / People / Directions /
 * Overview / … all portal-native); the route only carries the active person.
 */
export function PortalLayout() {
  const { replaceZone } = usePortalNavActions();
  // Pin the landing zone exactly once, when the viewer's manager status first
  // resolves: a manager lands on the Overview org rollup; an IC has no subtree,
  // so we leave the zone route-driven (null) → their own Person page. The rail
  // stays interactive while the status resolves, so honour a zone the user
  // already picked: a manager's choice is never overridden, and an IC who
  // landed on an org zone (now hidden for them) is reset to route-driven.
  const { isManager, isPending } = useViewerIsManager();
  const zone = usePortalZone();
  const landed = useRef(false);
  useEffect(() => {
    if (isPending || landed.current) return;
    landed.current = true;
    if (isManager) {
      if (zone == null) replaceZone("overview");
    } else if (zone != null && zone !== "person") {
      replaceZone(null);
    }
  }, [isPending, isManager, zone, replaceZone]);

  return (
    <SidebarProvider className="h-svh overflow-hidden">
      <PaneStateForLayout />
      <LensRail />
      <ContextPane />
      <SidebarInset className="min-w-0 overflow-x-clip overflow-y-auto">
        <MockBanner />
        {/* The impersonation indicator: it names whose data is on screen and
            carries the way out. Missing it left a view-as operator with no sign
            they were not looking at their own org, and no exit. */}
        <ViewAsBanner />
        <PortalTopBar />
        <ZoneContent />
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
