import type { Session } from "@/auth";

/**
 * Canonical authenticated-session fixture. Tests build sessions through this
 * so a new `Session` field lands in one place instead of breaking every
 * fixture literal (as adding `csrfToken` did — PR #216).
 */
export function makeSession(overrides: Partial<Session> = {}): Session {
  return {
    personId: "p-1",
    email: "bob.park@example.com",
    tenantId: "t-1",
    roles: ["user"],
    csrfToken: "csrf-1",
    // Unix seconds; far enough out that no test trips an accidental refresh.
    expiresAt: 4102444800, // 2100-01-01
    refreshAt: 4102444710,
    impersonatorEmail: null,
    ...overrides,
  };
}
