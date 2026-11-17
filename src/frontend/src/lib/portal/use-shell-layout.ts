import * as React from "react";

/**
 * How much chrome the viewport can afford. The portal shell sidebar holds a
 * 49px icon rail and a 256px context pane.
 *
 * - `wide` (>= 1024px): starts open, rail and pane beside the content.
 * - `narrow` (768–1023px): starts collapsed to the rail, keeping zone switching
 *   one tap away; the topbar trigger or a rail pick opens the pane.
 * - `phone` (< 768px): the kit renders the sidebar as a drawer, rail and pane
 *   together, behind the topbar trigger.
 *
 * The lower bound matches the kit's own mobile breakpoint (768): below it the
 * sidebar is a Sheet driven by `openMobile`, above it a panel driven by `open`.
 */
export type ShellLayout = "phone" | "narrow" | "wide";

const PHONE_MQ = "(max-width: 767px)";
const NARROW_MQ = "(min-width: 768px) and (max-width: 1023px)";

function subscribe(cb: () => void): () => void {
  const queries = [window.matchMedia(PHONE_MQ), window.matchMedia(NARROW_MQ)];
  for (const q of queries) q.addEventListener("change", cb);
  return () => {
    for (const q of queries) q.removeEventListener("change", cb);
  };
}

function getSnapshot(): ShellLayout {
  if (window.matchMedia(PHONE_MQ).matches) return "phone";
  if (window.matchMedia(NARROW_MQ).matches) return "narrow";
  return "wide";
}

// SSR fallback — assume the roomy case. Hydration corrects on first mount.
function getServerSnapshot(): ShellLayout {
  return "wide";
}

/** The layout tier the shell is in — one source, so rail and pane agree. */
export function useShellLayout(): ShellLayout {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
