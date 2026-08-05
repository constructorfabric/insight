import { afterEach, describe, expect, it } from "vitest";

import { authStore } from "./auth-store";
import { getViewerPersonId } from "./use-viewer";
import { makeSession } from "@/test/session";

afterEach(() => {
  authStore.reset();
});

describe("getViewerPersonId", () => {
  it("returns the session person id when authenticated", () => {
    authStore.setAuthenticated(makeSession({ personId: "p-1", tenantId: "", roles: [] }));

    expect(getViewerPersonId()).toBe("p-1");
  });

  it("returns null when there is no session", () => {
    authStore.setUnauthenticated();

    expect(getViewerPersonId()).toBeNull();
  });
});
