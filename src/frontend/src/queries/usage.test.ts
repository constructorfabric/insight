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

import { useUsagePeople, useUsageSummary } from "./usage";

describe("useUsageSummary", () => {
  it("re-reads rather than trusting the hour-long default", () => {
    useUsageSummary({ since: "2026-08-01", until: "2026-08-02" });

    expect(mocks.options?.staleTime).toBe(0);
    expect(mocks.options?.refetchOnMount).toBe("always");
  });
});

describe("useUsagePeople", () => {
  const AUGUST = { since: "2026-08-01", until: "2026-08-31" };
  const LATEST = { sort: "last_seen", direction: "desc" } as const;

  function useOptionsFor(...args: Parameters<typeof useUsagePeople>) {
    useUsagePeople(...args);
    return mocks.options ?? {};
  }

  type Placeholder = (previous: unknown, previousQuery?: { queryKey: unknown }) => unknown;

  it("asks again when the order changes", () => {
    const byDefault = useOptionsFor(AUGUST, null).queryKey;
    const byLastSeen = useOptionsFor(AUGUST, LATEST).queryKey;

    expect(byLastSeen).not.toEqual(byDefault);
  });

  it("keeps the rows on screen while the same period loads in a new order", () => {
    const previousQuery = { queryKey: useOptionsFor(AUGUST, null).queryKey };
    const placeholder = useOptionsFor(AUGUST, LATEST).placeholderData as Placeholder;
    const rows = { ...AUGUST, items: [] };

    expect(placeholder(rows, previousQuery)).toBe(rows);
  });

  it("drops the rows of another period rather than show them under this one", () => {
    const previousQuery = {
      queryKey: useOptionsFor({ since: "2026-07-01", until: "2026-07-31" }, LATEST).queryKey,
    };
    const placeholder = useOptionsFor(AUGUST, LATEST).placeholderData as Placeholder;

    const july = { since: "2026-07-01", until: "2026-07-31", items: [] };

    expect(placeholder(july, previousQuery)).toBeUndefined();
  });
});
