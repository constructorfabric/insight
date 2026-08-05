import type { AuthSnapshot, Session } from "./types";

let snapshot: AuthSnapshot = { status: "loading", session: null };

const listeners = new Set<() => void>();

function emit(): void {
  for (const fn of listeners) fn();
}

export const authStore = {
  subscribe(fn: () => void): () => void {
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  },

  getSnapshot(): AuthSnapshot {
    return snapshot;
  },

  setAuthenticated(session: Session): void {
    snapshot = { status: "authenticated", session };
    emit();
  },

  setUnauthenticated(): void {
    if (snapshot.status === "unauthenticated" && snapshot.session === null) return;
    snapshot = { status: "unauthenticated", session: null };
    emit();
  },

  reset(): void {
    snapshot = { status: "loading", session: null };
    emit();
  },
};
