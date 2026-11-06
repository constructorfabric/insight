import { useSyncExternalStore } from "react";

let shown = false;

const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function setMetricsViewPaneShown(next: boolean): void {
  shown = next;
  for (const listener of listeners) listener();
}

export function useMetricsViewPaneShown(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => shown,
    () => false,
  );
}
