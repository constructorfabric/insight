import { useSyncExternalStore } from "react";

/**
 * Portal PREFERENCES.
 *
 * `showPlanned` persists to localStorage (mirroring the metrics-v2 flag pattern
 * in feature-flags.ts). Nothing else lives here: every piece of navigation
 * state — zone, item, scope, slice, period — rides in the URL
 * (`portal-search.ts`, `portal-nav.ts`), because it describes the view rather
 * than the reader.
 *
 * The portal itself is no longer among them. It is the interface, not a choice,
 * so the key that used to carry the opt-out is DELETED on load rather than read
 * — a reader who once turned it off lands where everybody else does instead of
 * being held on screens nothing writes to any more.
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

/** Retired: read by nothing, removed wherever it is still stored. */
const RETIRED_ENABLED_KEY = "insight.portal";
const SHOW_PLANNED_KEY = "insight.portal.showPlanned";

interface PortalState {
  /** Whether navigation shows entries we have not built yet (`unbuilt` in the nav model). */
  showPlanned: boolean;
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

function readOptInPref(key: string): boolean {
  return readKey(key) === "true";
}

/**
 * Drops the retired opt-out, once, at load. Writing nothing when it is already
 * absent keeps this off the storage-quota path for every reader who never had
 * it.
 */
function forgetRetiredPreference(): void {
  if (typeof window === "undefined") return;
  if (readKey(RETIRED_ENABLED_KEY) === null) return;
  try {
    window.localStorage.removeItem(RETIRED_ENABLED_KEY);
  } catch {
    // localStorage unavailable — nothing reads the key either way.
  }
}

forgetRetiredPreference();

/**
 * Test-only door back to the screens the portal replaced.
 *
 * NOT a preference: nothing in the product writes it, no surface offers it,
 * and `insight.portal` — the key that used to — is deleted above. It exists
 * because the deployed-stand UI journeys are written against the legacy shell
 * (`tests/stand/ui/conftest.py` sets this before any app code runs); migrating
 * them is its own piece of work, and this is what keeps them running until
 * then. It goes when they do.
 *
 * Read ONCE, at load: the suite's init script precedes the first read and
 * nothing changes it afterwards, so there is nothing to subscribe to.
 */
const LEGACY_SHELL_KEY = "insight.legacyShell";
const legacyShell = readKey(LEGACY_SHELL_KEY) === "true";

/** Whether this document was told to render the pre-portal shell. */
export function readLegacyShell(): boolean {
  return legacyShell;
}

let state: PortalState = {
  showPlanned: readOptInPref(SHOW_PLANNED_KEY),
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

/** Show or hide the not-yet-built sections for this reader. */
export function setPortalShowPlanned(show: boolean): void {
  state = { ...state, showPlanned: show };
  persist(SHOW_PLANNED_KEY, show ? "true" : "false");
  emit();
}

/** Whether this reader wants planned sections listed. */
export function usePortalShowPlanned(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => state.showPlanned,
    () => false,
  );
}
