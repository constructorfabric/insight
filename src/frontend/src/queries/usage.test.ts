/**
 * Usage is written the moment someone clicks, and the app-wide default holds a
 * query fresh for an hour. Left on that default, stepping back to a period you
 * already looked at shows the snapshot from then — a month that reads smaller
 * than the week inside it.
 */
import { describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ options: null as Record<string, unknown> | null }));

vi.mock("@tanstack/react-query", () => ({
  useQuery: (options: Record<string, unknown>) => {
    mocks.options = options;
    return { data: undefined };
  },
}));

vi.mock("@/auth/use-auth", () => ({ useAuth: () => ({ session: null }) }));

import { useUsageSummary } from "./usage";

describe("useUsageSummary", () => {
  it("re-reads rather than trusting the hour-long default", () => {
    useUsageSummary({ since: "2026-08-01", until: "2026-08-02" });

    expect(mocks.options?.staleTime).toBe(0);
    expect(mocks.options?.refetchOnMount).toBe("always");
  });
});
