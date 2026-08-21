import { Settings2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { AppSidebarFooter } from "@/components/app-sidebar-footer";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";
import { useShellLayout } from "@/lib/portal/use-shell-layout";
import type { Zone } from "@/lib/portal/nav-model";
import { useZoneNav } from "@/lib/portal/use-zone-nav";
import { cn } from "@/lib/utils";

/**
 * The zone rail: one icon per zone, expanding to labels on hover.
 *
 * Zones that are dashboards in their own right (Person, People) link to the
 * existing dashboard routes and clear the theme-zone selection; other zones set
 * the active zone so the context pane switches. Zones the active role can't see
 * are filtered out (permission layer — FE stub over the future
 * role_section_visibility entity).
 *
 * Below 768px the rail renders nothing: 56px of icons plus a 256px pane left a
 * phone with ~60px of content. The same zones (labelled, not icon-only) live in
 * the context pane's drawer instead — see `ContextPane`. On a tablet the rail
 * stays: 56px is affordable, and it is the pane that collapses.
 *
 * ── The expansion, ported from the lite product's rail ──────────────────────
 *
 * Four things make it work, and each one is there because leaving it out broke
 * something:
 *
 * 1. The rail keeps its 56px slot in the layout and the labels open OVER the
 *    pane. Widening the element itself would shove the pane sideways every time
 *    a pointer crossed the rail on its way somewhere else.
 *
 * 2. The buttons widen to the full open width, but ONLY while it is open. A
 *    label you can read but not click is a trap: the pointer leaves the 56px
 *    column on its way to the word and the rail shuts before it arrives. People
 *    aim at what they can read. Keeping the buttons narrow while shut is what
 *    stops that from costing anything — approaching the pane from the content
 *    side never opens the rail over the row being reached for.
 *
 * 3. The buttons are inside the hover target, so a pointer resting on one keeps
 *    the rail open rather than fighting the thing that opened it.
 *
 * 4. A click collapses it until the pointer leaves. A click navigates, and the
 *    pointer is still on the rail afterwards — without this the rail reopens
 *    immediately, on top of the pane the click was aimed at. The lite product
 *    needs `sessionStorage` for this because its click reloads the page; here
 *    the navigation is client-side, so plain state survives it and is dropped
 *    the moment the pointer leaves.
 */

/**
 * How far the open rail reaches — wide enough for the longest zone label and no
 * wider. It deliberately does NOT cover the pane beside it: an overlay that
 * swallows the whole second column hides where the reader just was, and the
 * pane is what they are usually navigating towards. With an edge and a shadow
 * it reads as a panel resting over the pane rather than as a half-covered one.
 */
const OPEN_WIDTH = "12rem";

/**
 * Where the context pane ends, when it is there at all.
 *
 * The fade only has work to do while the pane is beside the rail. It collapses
 * off-canvas on the middle width tier, and a reader can shut it by hand on a
 * wide one — painting the full width of gradient in either case laid a dimmed
 * strip over the content for no reason.
 */
const PANE_EDGE = "calc(var(--rail-width) + var(--sidebar-width))";

/**
 * How long a pointer has to stay before the labels appear.
 *
 * The wait is the whole guard against opening over something a pointer was
 * only passing. An earlier version delayed the FADE and let the panel become
 * clickable immediately, which achieved the opposite of that: the invisible
 * panel is wide, a pointer crossing towards the pane landed on it, that
 * counted as still being inside, and the rail opened after the delay anyway —
 * over exactly the row being reached for, having swallowed any click made in
 * the meantime. Opening on a timer keeps "open" and "visible" the same fact,
 * so there is no window in which one is true and the other is not.
 */
const OPEN_AFTER_MS = 200;

export function LensRail() {
  const layout = useShellLayout();
  const { zones, activeZone, selectZone } = useZoneNav();
  const { state: paneState } = useSidebar();
  const paneIsBeside = paneState === "expanded";
  const [open, setOpen] = useState(false);
  // Suppresses the hover until the pointer leaves. Only a pointer-driven click
  // sets it — see the note where it is set.
  const [dismissed, setDismissed] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cancel = () => {
    if (timer.current !== null) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  };
  const close = () => {
    cancel();
    setOpen(false);
  };

  // A pointer can leave without a leave event: the element can be unmounted
  // under it by a width change, a dialog can take the pointer, the window can
  // lose focus. Each of those used to strand the state — cross the rail, narrow
  // the window to the phone tier and back, and it returned already open with
  // the pointer nowhere near it.
  //
  // Adjusted during render rather than in an effect. An effect would set state
  // after painting the stale frame, and React flags the cascade; this is the
  // documented shape for "a fact this state depended on has changed".
  const [layoutOfState, setLayoutOfState] = useState(layout);
  if (layoutOfState !== layout) {
    setLayoutOfState(layout);
    setOpen(false);
    setDismissed(false);
  }

  // Refs may not be touched during render, so the pending timer is dropped
  // here instead: without this a wait started before a width change would fire
  // afterwards and open a rail no pointer is on.
  useEffect(() => cancel, [layout]);

  if (layout === "phone") return null;

  return (
    <div
      data-testid="lens-rail"
      className="relative z-20 shrink-0"
      onPointerEnter={(e) => {
        // Touch fires enter at press and leave at release, so a tap would
        // flash the labels for the length of the tap and nothing else. Leave
        // it shut there; the same zones are labelled in the pane's drawer.
        if (e.pointerType === "touch" || dismissed) return;
        cancel();
        timer.current = setTimeout(() => setOpen(true), OPEN_AFTER_MS);
      }}
      // Keeps the labels while the keyboard is still in here. A pointer
      // crossing the rail on its way elsewhere is not a reason to collapse
      // under a focused button: the word on it would go to zero opacity and
      // leave a keyboard user on an unlabelled icon they cannot re-reveal
      // without a pointer. Blur is what ends a keyboard visit — see below.
      onPointerLeave={(e) => {
        if (!e.currentTarget.contains(document.activeElement)) close();
        setDismissed(false);
      }}
      // Keyboard gets the labels too, and immediately: a sighted keyboard user
      // was tabbing through eight identical icons with the text at zero
      // opacity, and `title` does not surface on focus in any browser.
      onFocusCapture={() => {
        cancel();
        setOpen(true);
      }}
      onBlurCapture={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
          close();
          setDismissed(false);
        }
      }}
    >
      <Sidebar
        collapsible="none"
        className="w-(--rail-width)! overflow-visible border-e"
      >
        {/* A fade beside the panel, not a flat veil.
            The pane's rows do not object to being dimmed — they object to
            being CUT, and a hard edge through the middle of a word reads as a
            rendering fault whatever its brightness. So the strip the panel
            does not cover goes from fully hidden at the panel's edge to fully
            visible at the pane's, and a row dissolves instead of stopping
            mid-letter. Dimming it uniformly was tried first and did nothing:
            the cut, not the contrast, was the problem. */}
        {paneIsBeside ? (
        <div
          aria-hidden
          className={cn(
            // Absolute, like everything else in the rail: `fixed` worked only
            // because the rail happens to sit at the viewport's edge and no
            // ancestor establishes a containing block, neither of which is
            // enforced anywhere.
            "pointer-events-none absolute inset-y-0 transition-opacity duration-150",
            // From the panel's own colour, not the page background — the two
            // tokens differ, most visibly in dark theme, and starting from the
            // wrong one puts a step exactly at the seam. Mirrored for RTL:
            // `insetInlineStart` is logical while a gradient direction is not,
            // so the opaque end landed away from the panel and reinstated the
            // hard cut this exists to remove.
            "bg-gradient-to-r from-sidebar to-transparent rtl:bg-gradient-to-l",
            open ? "opacity-100" : "opacity-0"
          )}
          style={{
            insetInlineStart: OPEN_WIDTH,
            width: `calc(${PANE_EDGE} - ${OPEN_WIDTH})`,
          }}
        />
        ) : null}
        {/* The panel the labels sit on.
            It takes pointer events WHILE OPEN, and that is not a detail: with
            it inert the gaps between buttons belong to whatever is underneath,
            so a pointer moving from an icon towards its label crosses bare
            panel, the rail counts that as having been left, and it slams shut
            under the hand that was reaching for it.
            Delayed both ways: crossing the rail on the way elsewhere should not
            flash it open, and leaving briefly should not shut it. */}
        <div
          aria-hidden={!open}
          className={cn(
            "absolute inset-y-0 start-0 border-e bg-sidebar transition-opacity duration-150",
            open
              ? "pointer-events-auto opacity-100 shadow-lg"
              : "pointer-events-none opacity-0"
          )}
          style={{ width: OPEN_WIDTH }}
          onClick={close}
        />
        <SidebarHeader className="relative z-10 items-start ps-3">
          <div className="flex size-8 items-center justify-center rounded-md bg-sidebar-primary text-sm font-bold text-sidebar-primary-foreground">
            I
          </div>
        </SidebarHeader>
        {/* The zone list scrolls while shut and lets the labels out while
            open. It cannot do both: a box that clips its overflow on one axis
            clips it on the other too, whatever `overflow-x: visible` says, so
            the widened buttons were being cut off at the rail's edge.
            Scrolling is the half worth losing, and only for as long as the
            labels are showing — a reader who needs to scroll can move the
            pointer away, which is also how they stop reading the labels. An
            earlier version escaped the clip with a blanket child selector,
            which won on specificity against this box's own overflow and left
            the list unable to scroll at all, open or shut. */}
        <SidebarContent className={open ? "overflow-visible" : undefined}>
          <SidebarMenu className="items-start gap-1 ps-2">
            {zones.map((z) => (
              <ZoneItem
                key={z.id}
                zone={z}
                active={activeZone === z.id}
                open={open}
                onSelect={(zone, viaPointer) => {
                  if (viaPointer) {
                    setDismissed(true);
                    close();
                  }
                  selectZone(zone);
                }}
              />
            ))}
          </SidebarMenu>
        </SidebarContent>
        <SidebarFooter className="relative z-10 items-start gap-1 ps-2">
          <Popover open={settingsOpen} onOpenChange={setSettingsOpen}>
            <PopoverTrigger
              render={
                <button
                  type="button"
                  title="Settings"
                  className="flex size-10 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
                >
                  <Settings2 className="size-[19px]" aria-hidden />
                  <span className="sr-only">Settings</span>
                </button>
              }
            />
            <PopoverContent
              side="right"
              align="end"
              className="w-64 gap-0 p-1"
              // Handing focus back to the trigger reopens the rail through
              // `onFocusCapture`, over the surface just asked for.
              finalFocus={false}
            >
              <AppSidebarFooter
                onNavigate={() => {
                  setSettingsOpen(false);
                  // Note 4 above, reached from the footer rather than a zone.
                  setDismissed(true);
                  close();
                }}
              />
            </PopoverContent>
          </Popover>
        </SidebarFooter>
      </Sidebar>
    </div>
  );
}

function ZoneItem({
  zone,
  active,
  open,
  onSelect,
}: {
  zone: Zone;
  active: boolean;
  open: boolean;
  onSelect: (zone: Zone, viaPointer: boolean) => void;
}) {
  const Icon = zone.icon;

  return (
    <SidebarMenuItem className="relative z-10">
      <SidebarMenuButton
        isActive={active}
        title={zone.label}
        // 40px shut, the full open width while open — see note 2 above. The
        // icon does not move between the two: the button starts its content at
        // the same offset either way.
        className={cn(
          "h-10 justify-start gap-2 overflow-hidden p-0 ps-[10px] transition-[width] duration-150",
          open || "w-10"
        )}
        style={open ? { width: `calc(${OPEN_WIDTH} - 1rem)` } : undefined}
        // `detail` counts pointer clicks: keyboard activation reports 0.
        onClick={(e) => onSelect(zone, e.detail > 0)}
      >
        <Icon className="shrink-0" />
        {/* Visible only while open, and never a pointer target of its own —
            the button under it is what widens, so the word IS the hit area. */}
        <span
          className={cn(
            "truncate transition-opacity duration-150",
            open ? "opacity-100" : "opacity-0"
          )}
        >
          {zone.label}
        </span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
