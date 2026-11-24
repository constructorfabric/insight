import { describe, expect, it } from "vitest";

import { sessionAuthorizationScope } from "@/auth/session-scope";
import type { Session } from "@/auth/types";

describe("sessionAuthorizationScope", () => {
  it("returns null without a session and canonicalizes role order", () => {
    expect(sessionAuthorizationScope(null)).toBeNull();
    expect(
      sessionAuthorizationScope({
        tenantId: "tenant",
        personId: "person",
        email: "person@example.com",
        impersonatorEmail: null,
        roles: ["viewer", "admin"],
        csrfToken: "csrf",
        expiresAt: 1,
        refreshAt: 1,
      } satisfies Session)
    ).toBe(
      JSON.stringify({
        tenantId: "tenant",
        personId: "person",
        impersonatorEmail: null,
        roles: ["admin", "viewer"],
      })
    );
  });
});
