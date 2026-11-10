import type { KeyboardEvent } from "react";

const COMPOSITE_NAVIGATION_KEYS = new Set([
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "ArrowDown",
  "Home",
  "End",
]);

// WORKAROUND: a popover is portalled out of the DOM but not out of the React
// tree, so its key events bubble to an enclosing ToggleGroup, whose roving focus takes them.
export function keepNavigationKeysInside(event: KeyboardEvent<HTMLElement>) {
  if (COMPOSITE_NAVIGATION_KEYS.has(event.key)) event.stopPropagation();
}
