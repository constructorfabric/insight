import { useSidebar } from "@gears-frontx/ui-kit";
import { useShellLayout } from "@/lib/portal/use-shell-layout";

/**
 * Dismiss the mobile drawer after a LEAF pick (a section / lens / group): on a
 * phone the pane is the drawer, so leaving it open would hide the very view the
 * reader just chose. Zone picks deliberately keep it open — the zone's items
 * render right below, so zone-then-item is one pass. No-op on desktop, where
 * the pane is always-visible chrome.
 */
export function useDismissDrawer(): () => void {
  const layout = useShellLayout();
  const { setOpen, setOpenMobile } = useSidebar();
  return () => {
    // Below 768 the sidebar is a Sheet (`openMobile`); on a tablet it collapses
    // to the rail (`open`). Wide keeps the pane open — nothing to dismiss.
    if (layout === "phone") setOpenMobile(false);
    else if (layout === "narrow") setOpen(false);
  };
}
