import { TooltipProvider } from "@gears-frontx/ui-kit";
import { createElement, type ReactNode } from "react";

/** The app mounts one tooltip provider at its root; tests use it with no delay. */
export function AppTooltips({ children }: { children: ReactNode }) {
  return createElement(TooltipProvider, { delay: 0 }, children);
}

/**
 * The open tooltip's popup. The kit marks its positioner, popup and arrow with
 * the side it opened on; the popup is the one inside the positioner that is
 * not the decorative arrow. Menus, popovers and selects share that shape but
 * carry a role, which a tooltip popup does not.
 */
export function openTooltip(): Element | null {
  const marked = [...document.querySelectorAll("[data-open][data-side]")];
  return (
    marked.find(
      (element) =>
        element.parentElement?.hasAttribute("data-side") === true &&
        !element.hasAttribute("aria-hidden") &&
        !element.hasAttribute("role")
    ) ?? null
  );
}
