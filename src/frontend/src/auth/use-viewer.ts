import { useSyncExternalStore } from "react";

import { authStore } from "./auth-store";

/**
 * The current viewer, derived from the session summary (`/auth/me`).
 * `personId` (the gateway JWT `sub`) is the SPA's person key since the identity
 * cutover — routes, metric requests and profile lookups all use it. `email`
 * stays for display and for the email-shaped protocols (`__override`).
 */
export type Viewer = {
  email: string | null;
  personId: string | null;
};

function resolve(): Viewer {
  const { session } = authStore.getSnapshot();
  return {
    email: session?.email ?? null,
    personId: session?.personId ?? null,
  };
}

export function useViewer(): Viewer {
  useSyncExternalStore(
    authStore.subscribe,
    authStore.getSnapshot,
    authStore.getSnapshot,
  );
  return resolve();
}


export function getViewerPersonId(): string | null {
  return resolve().personId;
}
