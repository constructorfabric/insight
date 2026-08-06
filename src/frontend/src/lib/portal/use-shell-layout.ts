import * as React from "react";

/**
 * How much chrome the viewport can afford. The portal shell has two fixed
 * sidebars — a 56px icon rail and a 256px context pane — which is fine on a
 * wide screen and ruinous on a narrow one.
 *
 * - `wide` (>= 1024px): both in normal flow, as designed.
 * - `narrow` (768–1023px): the rail stays (56px is cheap and keeps zone
 *   switching one tap away), the pane collapses off-canvas behind the topbar
 *   trigger. 312px of chrome on a 900px window left the content 588px.
 * - `phone` (< 768px): neither fits. The rail hides and the pane becomes the
 *   only navigation surface, so it also carries the rail's zone list and
 *   settings menu.
 *
 * The lower bound deliberately matches `useIsMobile` (768), because that is the
 * breakpoint at which the sidebar primitive itself switches from an off-canvas
 * panel to a Sheet — one boundary, two consequences, no third opinion. Which
 * collapse mechanism is live matters to callers: below 768 the pane is a Sheet
 * driven by `openMobile`, above it an off-canvas panel driven by `open`.
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
