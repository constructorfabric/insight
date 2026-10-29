import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Outlet } from "@tanstack/react-router";

import { AppBootSpinner } from "@/components/app-boot-spinner";
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
import { usePortalNavActions, usePortalZone } from "@/lib/portal/portal-nav";
import {
  landingDecision,
  type LandingDecision,
} from "@/lib/portal/landing-zone";
import { zoneHidden, zonePlanned } from "@/lib/portal/nav-policy";
import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import {
  useShellLayout,
  type ShellLayout,
} from "@/lib/portal/use-shell-layout";
import { useViewerReach } from "@/lib/portal/use-viewer-reach";
import { useIsAdmin } from "@/queries/identity-me";

/**
 * Portal shell (Phase 1 buildout), rendered unless the reader opts out via
 * `insight.portal`. Composition (one SidebarProvider, all normal flow):
 *   [ lens rail ] [ zone-contextual pane ] [ content ]
 * Every zone renders through `<ZoneContent/>` (Person / People / Directions /
 * Overview / … all portal-native); the route only carries the active person.
 */
export function PortalLayout() {
  const { replaceZone } = usePortalNavActions();
  // Pin the landing zone exactly once, when the viewer's shape resolves: a
  // manager lands on the Overview org rollup; an IC has no subtree, so their
  // zone stays route-driven (their own Person page) — EXCEPT Manage, which
  // the admin role opens regardless of reports. The rules live in
  // `landingDecision` (pure, table-tested); this effect only applies them.
  const { canSeeOthers, isPending } = useViewerReach();
  const {
    isAdmin,
    isPending: adminPending,
    isError: adminError,
  } = useIsAdmin();
  const showPlanned = usePortalShowPlanned();
  const zone = usePortalZone();
  // INVARIANT: the decision is applied once; it settles to "keep" when the URL
  // matches it, so a later visit to a bare /portal is the reader's own.
  const [landing, setLanding] = useState<LandingDecision["kind"] | null>(null);
  const decision = landingDecision({
    zone,
    mgrPending: isPending,
    canSeeOthers,
    // An errored check is "unknown", not "no": the landing decision is
    // one-shot, so resetting on it would permanently rewrite a URL an
    // admin deliberately opened. Waiting costs nothing — the me query
    // retries itself until an answer lands.
    adminPending: adminPending || adminError,
    isAdmin,
    overviewVisible:
      !zoneHidden("overview") && (!zonePlanned("overview") || showPlanned),
  });
  if (landing === null && decision.kind !== "wait") setLanding(decision.kind);
  if (landing === "pin-overview" && zone != null) setLanding("keep");
  if (landing === "reset" && zone == null) setLanding("keep");

  useEffect(() => {
    if (landing === "pin-overview") replaceZone("overview");
    if (landing === "reset") replaceZone(null);
  }, [landing, replaceZone]);

  // A bare /portal renders the Person view, so it waits for the landing
  // decision rather than showing a page it is about to leave.
  const settling = (landing ?? decision.kind) !== "keep";
  if (zone == null && settling) return <AppBootSpinner />;

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
      <SidebarInset className="isolate min-w-0 overflow-x-clip overflow-y-auto">
        <MockBanner />
        {/* The impersonation indicator: it names whose data is on screen and
            carries the way out. Missing it left a view-as operator with no sign
            they were not looking at their own org, and no exit. */}
        <ViewAsBanner />
        <PortalTopBar />
        <Outlet />
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
