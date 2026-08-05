import { useSyncExternalStore } from "react";

import type { FocusMode } from "@/lib/peers";

const FOCUS_KEY = "insight.focus-mode";
const EXPLANATIONS_KEY = "insight.explanations";

const VALID_FOCUS: ReadonlySet<FocusMode> = new Set([
  "all",
  "critical",
  "rewards",
  "neutral",
]);

interface SettingsState {
  focusMode: FocusMode;
  showExplanations: boolean;
}

const DEFAULT_STATE: SettingsState = {
  focusMode: "all",
  showExplanations: false,
};

function readFocusMode(): FocusMode {
  if (typeof window === "undefined") return DEFAULT_STATE.focusMode;
  const raw = window.localStorage.getItem(FOCUS_KEY);
  if (raw && VALID_FOCUS.has(raw as FocusMode)) return raw as FocusMode;
  return DEFAULT_STATE.focusMode;
}

function readExplanations(): boolean {
  if (typeof window === "undefined") return DEFAULT_STATE.showExplanations;
  return window.localStorage.getItem(EXPLANATIONS_KEY) === "true";
}

let state: SettingsState = {
  focusMode: readFocusMode(),
  showExplanations: readExplanations(),
};

const listeners = new Set<() => void>();

function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

function getSnapshot(): SettingsState {
  return state;
}

function setState(next: Partial<SettingsState>): void {
  state = { ...state, ...next };
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(FOCUS_KEY, state.focusMode);
      window.localStorage.setItem(
        EXPLANATIONS_KEY,
        state.showExplanations ? "true" : "false",
      );
    } catch {
      // localStorage may be unavailable; in-memory state still updated.
    }
  }
  for (const fn of listeners) fn();
}

export function useSettings(): {
  focusMode: FocusMode;
  showExplanations: boolean;
  setFocusMode: (mode: FocusMode) => void;
  setShowExplanations: (show: boolean) => void;
} {
  const snap = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return {
    focusMode: snap.focusMode,
    showExplanations: snap.showExplanations,
    setFocusMode: (focusMode) => setState({ focusMode }),
    setShowExplanations: (showExplanations) => setState({ showExplanations }),
  };
}
