import { useSyncExternalStore } from "react";

/**
 * Portal PREFERENCES (feature-flagged behind `insight.portal`).
 *
 * `enabled` and `showPlanned` persist to localStorage (mirroring the metrics-v2
 * flag pattern in feature-flags.ts). Nothing else lives here: every piece of
 * navigation state — zone, item, scope, slice, period — rides in the URL
 * (`portal-search.ts`, `portal-nav.ts`), because it describes the view rather
 * than the reader.
 */

/**
 * Org scope — WHO is counted in every org zone (design §6). `root` is the
 * person id of a manager node inside the viewer's subtree (null = the viewer's
 * whole org); `directOnly` narrows to direct reports. Phase 2 reserves
 * `attrFilter` (attribute-value cut across the tree) — no UI yet.
 */
export interface OrgScope {
  root: string | null;
  directOnly: boolean;
  attrFilter?: { key: string; value: string };
}

const ENABLED_KEY = "insight.portal";
const SHOW_PLANNED_KEY = "insight.portal.showPlanned";

interface PortalState {
  enabled: boolean;
  /**
   * Whether navigation shows entries we have not built yet (`unbuilt` in the
   * nav model). Default ON while the whole portal is itself a preview: for us
   * and for demos the dead entries ARE the roadmap. Turn it off — or flip the
   * default — the day the portal stops being opt-in, so a customer never has
   * to tell our backlog apart from their own missing data.
   */
  showPlanned: boolean;
}

/** Router-safe read: `beforeLoad` runs outside React, so it cannot use a hook. */
export function readPortalEnabled(): boolean {
  return readEnabled();
}

/**
 * Reading storage can THROW, not just return null: a sandboxed iframe or
 * blocked third-party storage raises SecurityError on property access. These
 * readers run at module scope, so an unguarded throw escapes before any
 * component mounts and takes the whole bundle down over a preview flag.
 */
function readKey(key: string): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function readEnabled(): boolean {
  return readKey(ENABLED_KEY) === "true";
}

/** Absent key = default ON (see `showPlanned`); only an explicit "false" hides. */
function readShowPlanned(): boolean {
  return readKey(SHOW_PLANNED_KEY) !== "false";
}

let state: PortalState = {
  enabled: readEnabled(),
  showPlanned: readShowPlanned(),
};

const listeners = new Set<() => void>();

function emit(): void {
  for (const fn of listeners) fn();
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

function persist(key: string, value: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // localStorage unavailable — in-memory state still updated.
  }
}

/** Turn the portal preview on or off for this reader. */
export function setPortalEnabled(enabled: boolean): void {
  state = { ...state, enabled };
  persist(ENABLED_KEY, enabled ? "true" : "false");
  emit();
}

/** Show or hide the not-yet-built sections for this reader. */
export function setPortalShowPlanned(show: boolean): void {
  state = { ...state, showPlanned: show };
  persist(SHOW_PLANNED_KEY, show ? "true" : "false");
  emit();
}

/** Whether this reader has the portal preview on. */
export function usePortalEnabled(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => state.enabled,
    () => false,
  );
}

/** Whether this reader wants planned sections listed. */
export function usePortalShowPlanned(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => state.showPlanned,
    () => true,
  );
}
