import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, expect, it, vi } from "vitest";

import {
  getPerson,
  searchPersons,
  IdentityApiError,
} from "@/api/identity-client";
import type { PersonSearchResponse } from "@/api/identity-client";
import { useConnectorScheduleAuthor } from "./connector-schedule-author";

const auth = vi.hoisted(() => ({
  scope: "example-session",
  me: { person_id: "00000000-0000-4000-8000-000000000001" } as
    { person_id: string } | undefined,
}));
vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: auth.scope }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: (session: string) => session,
}));
vi.mock("@/queries/identity-me", () => ({
  useMe: () => ({ data: auth.me }),
}));
vi.mock("@/api/identity-client", async (original) => ({
  ...(await original<typeof import("@/api/identity-client")>()),
  getPerson: vi.fn(),
  searchPersons: vi.fn(),
}));

const OTHER = "00000000-0000-4000-8000-000000000002";
const person = {
  person_id: OTHER,
  display_name: "Example Admin",
};
function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { client, wrapper };
}
beforeEach(() => {
  vi.resetAllMocks();
  auth.scope = "example-session";
  auth.me = { person_id: "00000000-0000-4000-8000-000000000001" };
  vi.mocked(getPerson).mockRejectedValue(new IdentityApiError(404, null));
});

it("names an admin outside the viewer’s visible roster and caches the tenant lookup", async () => {
  let finish!: (value: PersonSearchResponse) => void;
  vi.mocked(searchPersons).mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    })
  );
  const { client, wrapper } = setup();
  const hook = renderHook(
    () => [
      useConnectorScheduleAuthor(OTHER),
      useConnectorScheduleAuthor(OTHER),
    ],
    { wrapper }
  );
  expect(hook.result.current).toEqual(["another admin", "another admin"]);
  await act(async () => {
    finish({ items: [person] });
  });
  await waitFor(() =>
    expect(hook.result.current).toEqual(["Example Admin", "Example Admin"])
  );
  expect(searchPersons).toHaveBeenCalledTimes(1);
  expect(searchPersons).toHaveBeenCalledWith(
    OTHER,
    { limit: 1 },
    expect.any(AbortSignal)
  );
  expect(getPerson).not.toHaveBeenCalled();
  hook.unmount();
  const reopened = renderHook(() => useConnectorScheduleAuthor(OTHER), {
    wrapper,
  });
  expect(reopened.result.current).toBe("Example Admin");
  expect(searchPersons).toHaveBeenCalledTimes(1);
  reopened.unmount();
  client.clear();
});

it("says you for the viewer without a person lookup", () => {
  const { client, wrapper } = setup();
  const hook = renderHook(
    () => useConnectorScheduleAuthor("00000000-0000-4000-8000-000000000001"),
    { wrapper }
  );
  expect(hook.result.current).toBe("you");
  expect(getPerson).not.toHaveBeenCalled();
  expect(searchPersons).not.toHaveBeenCalled();
  hook.unmount();
  client.clear();
});

it.each([403, 404, 500])(
  "keeps another admin after lookup failure %s, never the id",
  async (status) => {
    vi.mocked(searchPersons).mockRejectedValue(
      new IdentityApiError(status, null)
    );
    const { client, wrapper } = setup();
    const hook = renderHook(() => useConnectorScheduleAuthor(OTHER), {
      wrapper,
    });
    await waitFor(() =>
      expect(client.getQueryCache().getAll()[0].state.status).toBe("error")
    );
    expect(hook.result.current).toBe("another admin");
    expect(hook.result.current).not.toContain(OTHER);
    hook.unmount();
    client.clear();
  }
);

it("uses another admin for a blank name and isolates a new viewer's cache", async () => {
  vi.mocked(searchPersons)
    .mockResolvedValueOnce({ items: [person] })
    .mockResolvedValue({ items: [{ ...person, display_name: "  " }] });
  const { client, wrapper } = setup();
  const hook = renderHook(() => useConnectorScheduleAuthor(OTHER), { wrapper });
  await waitFor(() => expect(hook.result.current).toBe("Example Admin"));
  auth.scope = "another-session";
  hook.rerender();
  expect(hook.result.current).toBe("another admin");
  await waitFor(() => expect(searchPersons).toHaveBeenCalledTimes(2));
  expect(hook.result.current).toBe("another admin");
  hook.unmount();
  client.clear();
});

it.each(["00000000-0000-4000-8000-000000000001", OTHER])(
  "waits for the viewer before looking up %s",
  async (personId) => {
    auth.me = undefined;
    vi.mocked(searchPersons).mockResolvedValue({ items: [person] });
    const { client, wrapper } = setup();
    const hook = renderHook(() => useConnectorScheduleAuthor(personId), {
      wrapper,
    });
    expect(hook.result.current).toBe("another admin");
    expect(searchPersons).not.toHaveBeenCalled();
    expect(getPerson).not.toHaveBeenCalled();

    auth.me = { person_id: "00000000-0000-4000-8000-000000000001" };
    hook.rerender();
    if (personId === auth.me.person_id) {
      expect(hook.result.current).toBe("you");
      expect(searchPersons).not.toHaveBeenCalled();
    } else {
      await waitFor(() => expect(hook.result.current).toBe("Example Admin"));
    }
    hook.unmount();
    client.clear();
  }
);

it.each([
  { items: [] },
  { items: [{ person_id: OTHER, display_name: "  " }] },
  { items: [{ ...person, person_id: "00000000-0000-4000-8000-000000000003" }] },
])(
  "keeps another admin when the search finds no matching name: %j",
  async (found) => {
    vi.mocked(searchPersons).mockResolvedValue(found);
    const { client, wrapper } = setup();
    const hook = renderHook(() => useConnectorScheduleAuthor(OTHER), {
      wrapper,
    });
    await waitFor(() =>
      expect(client.getQueryCache().getAll()[0].state.status).toBe("success")
    );
    expect(hook.result.current).toBe("another admin");
    expect(hook.result.current).not.toContain(OTHER);
    hook.unmount();
    client.clear();
  }
);
