import { useEffect, useRef, type CSSProperties } from "react";
import { Outlet } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { MockBanner } from "@/components/mock-banner";
import { ViewAsBanner } from "@/components/view-as-banner";
import { ContextPane, ZonePages } from "@/components/portal/context-pane";
import {
  LensRail,
  RailFooterItems,
  RailItems,
} from "@/components/portal/lens-rail";
import { PortalTopBar } from "@/components/portal/portal-topbar";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarInset,
  SidebarProvider,
  useSidebar,
} from "@gears-frontx/ui-kit";
import { useMetricsViewHidesPane } from "@/lib/portal/metrics-view-pane";
import { PortalDocumentTitle } from "@/lib/portal/document-title-frame";
import { usePortalNavActions, usePortalZone } from "@/lib/portal/portal-nav";
import { bareLandingZone, landingDecision } from "@/lib/portal/landing-zone";
import {
  usePortalRailLabels,
  usePortalShowPlanned,
} from "@/lib/portal/portal-store";
import {
  useShellLayout,
  type ShellLayout,
} from "@/lib/portal/use-shell-layout";
import { useViewerReach } from "@/lib/portal/use-viewer-reach";
import { useZoneNav } from "@/lib/portal/use-zone-nav";
import { useIsAdmin } from "@/queries/identity-me";

/**
 * Portal shell (Phase 1 buildout), rendered unless the reader opts out via
 * `insight.portal`. Composition: one SidebarProvider, one collapsible shell
 * sidebar holding two columns, then the content:
 *   [ lens rail | zone-contextual pane ] [ content ]
 * Every zone renders through `<ZoneContent/>` (Person / People / Directions /
 * Overview / … all portal-native); the route only carries the active person.
 */
export function PortalLayout() {
  const { replaceZone } = usePortalNavActions();
  // Leave an org zone the viewer cannot see, once their shape resolves. The
  // rules live in `landingDecision` (pure, table-tested); this effect only
  // applies them, once, so a zone the reader picks later is never rewritten.
  const { canSeeOthers, isPending } = useViewerReach();
  const {
    isAdmin,
    isPending: adminPending,
    isError: adminError,
  } = useIsAdmin();
  const zone = usePortalZone();
  const homeShown = bareLandingZone(usePortalShowPlanned()) === "overview";
  const landed = useRef(false);
  const paneHidden = useMetricsViewHidesPane();
  const railWidth = usePortalRailLabels() ? RAIL_WIDTH_LABELS : RAIL_WIDTH_ICONS;
  useEffect(() => {
    if (landed.current) return;
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
      homeShown,
    });
    if (decision.kind === "wait") return;
    landed.current = true;
    if (decision.kind === "reset") replaceZone(null);
  }, [
    isPending,
    canSeeOthers,
    adminPending,
    adminError,
    isAdmin,
    homeShown,
    zone,
    replaceZone,
  ]);

  return (
    <PortalDocumentTitle>
      <SidebarProvider
        className="h-svh overflow-hidden"
        // Rail plus pane: the provider's width is the whole open sidebar.
        style={
          {
            "--rail-width": railWidth,
            "--sidebar-width": paneHidden
              ? railWidth
              : `calc(${railWidth} + ${PANE_WIDTH})`,
          } as CSSProperties
        }
      >
        <PaneStateForLayout />
        <ShellSidebar />
        {/* INVARIANT: `isolate` keeps the inset's own stacking — the sticky
            topbar included — under the fixed shell sidebar. */}
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
    </PortalDocumentTitle>
  );
}

const RAIL_WIDTH_ICONS = "calc(var(--sidebar-width-icon) + 1px)";
const RAIL_WIDTH_LABELS = "12rem";
const PANE_WIDTH = "16rem";

/**
 * Collapsible to the icon rail: the rail column stays, the pane beside it goes.
 * On a phone the kit renders it as a drawer holding one labelled column.
 */
export function ShellSidebar() {
  const { isMobile } = useSidebar();

  return (
    <Sidebar
      collapsible="icon"
      className={
        // WORKAROUND: the kit's phone drawer is a Sheet, whose 24px inset pads
        // the sidebar inside it; the sidebar's rows carry their own spacing.
        "overflow-hidden md:*:flex-row [--sheet-inset:0]"
      }
    >
      {isMobile ? (
        <PhoneDrawer />
      ) : (
        <>
          <LensRail />
          <ContextPane />
        </>
      )}
    </Sidebar>
  );
}

/** The rail items as labelled rows, the active item's pages under them. */
function PhoneDrawer() {
  const { t } = useTranslation();
  const nav = useZoneNav();

  return (
    <>
      <SidebarContent>
        <RailItems />
        <div
          role="navigation"
          aria-label={t("common.a11y.section_navigation")}
          className="contents"
        >
          <ZonePages nav={nav} />
        </div>
      </SidebarContent>
      <SidebarFooter>
        <RailFooterItems />
      </SidebarFooter>
    </>
  );
}

/**
 * The pane is open by default only on a wide screen; narrower, the shell
 * sidebar must START collapsed to its rail, or a tablet is back to 305px of
 * chrome. The provider's `open` defaults to true and survives a resize, so a
 * layout change has to reset it.
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
