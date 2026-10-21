vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchShares: vi.fn(),
    shareDashboard: vi.fn(),
    unshareDashboard: vi.fn(),
  };
});
vi.mock("@/api/identity-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/identity-client")>();
  return { ...actual, listRoleHolders: vi.fn() };
});
const people = vi.hoisted(() => {
  const OWNER = "01900000-0000-7000-8000-0000000000a0";
  const BOB = "01900000-0000-7000-8000-0000000000b0";
  const CAROL = "01900000-0000-7000-8000-0000000000c0";
  const DAVE = "01900000-0000-7000-8000-0000000000d0";
  const names: Record<string, string> = {
    [OWNER]: "Ann Owner",
    [BOB]: "Bob Park",
    [CAROL]: "Carol Chen",
    [DAVE]: "Dave Liu",
  };

  return {
    ids: { OWNER, BOB, CAROL, DAVE },
    names,
    found: [] as { person_id: string; display_name: string }[],
  };
});
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  usePersonList: () => ({
    data: { pages: [{ items: people.found }] },
    isFetching: false,
    isFetchingNextPage: false,
    isPlaceholderData: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  }),
}));
vi.mock("@/queries/person-name", () => ({
  usePersonName: (personId: string | null) => ({
    isPending: false,
    data: people.names[personId ?? ""] ?? null,
  }),
}));
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import * as identityClient from "@/api/identity-client";

import { ShareDashboard } from "./share-dashboard";

const { OWNER, BOB, CAROL, DAVE } = people.ids;

const everyone = [OWNER, BOB, CAROL, DAVE].map((person_id) => ({
  person_id,
  display_name: people.names[person_id] ?? "",
}));

function admins(personIds: string[], complete = true) {
  vi.mocked(identityClient.listRoleHolders).mockResolvedValue({
    personIds: new Set(personIds),
    complete,
  });
}

function renderDialog() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ShareDashboard
        id="dash-1"
        title="Delivery"
        ownerPersonId={OWNER}
        open
        onClose={vi.fn()}
      />
    </QueryClientProvider>
  );

  return userEvent.setup();
}

const sharedWith = () =>
  screen.getByRole("region", { name: "Shared with" });

async function search(user: ReturnType<typeof userEvent.setup>) {
  await user.type(await screen.findByRole("searchbox"), "an");
}

beforeEach(() => {
  vi.resetAllMocks();
  people.found = everyone;
  admins([OWNER, BOB, CAROL]);
  vi.mocked(customClient.fetchShares).mockResolvedValue([
    { person_id: CAROL, granted_by: OWNER, created_at: "2026-09-01T09:00:00Z" },
  ]);
  vi.mocked(customClient.shareDashboard).mockResolvedValue(undefined);
  vi.mocked(customClient.unshareDashboard).mockResolvedValue(undefined);
});

describe("<ShareDashboard>", () => {
  it("lists who it is shared with, by name", async () => {
    renderDialog();

    expect(
      await within(sharedWith()).findByText("Carol Chen")
    ).toBeInTheDocument();
    expect(customClient.fetchShares).toHaveBeenCalledWith("dash-1");
  });

  it("withdraws a share from its row", async () => {
    const user = renderDialog();

    await user.click(
      await screen.findByRole("button", { name: "Stop sharing with Carol Chen" })
    );

    expect(customClient.unshareDashboard).toHaveBeenCalledWith(
      "dash-1",
      CAROL
    );
  });

  it("offers only administrators, never the owner or someone it is already shared with", async () => {
    const user = renderDialog();
    await within(sharedWith()).findByText("Carol Chen");

    await search(user);

    expect(screen.getByRole("button", { name: "Bob Park" })).toBeInTheDocument();
    for (const name of ["Ann Owner", "Carol Chen", "Dave Liu"]) {
      expect(
        screen.queryByRole("button", { name }),
        `should not offer: ${name}`
      ).toBeNull();
    }
  });

  it("shares with the administrator picked", async () => {
    const user = renderDialog();
    await search(user);

    await user.click(screen.getByRole("button", { name: "Bob Park" }));

    expect(customClient.shareDashboard).toHaveBeenCalledWith("dash-1", BOB);
  });

  it("shows what the service said when it refuses a share", async () => {
    vi.mocked(customClient.shareDashboard).mockRejectedValue(
      new CustomApiError(400, {
        detail: "Request validation failed",
        context: {
          field_violations: [
            { field: "person_id", description: "not a current administrator" },
          ],
        },
      })
    );
    const user = renderDialog();
    await search(user);

    await user.click(screen.getByRole("button", { name: "Bob Park" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "not a current administrator"
    );
  });

  it("refuses to offer anyone when the administrator listing may be cut short", async () => {
    admins([OWNER, BOB], false);
    renderDialog();

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't read the administrators."
    );
    await waitFor(() => expect(screen.queryByRole("searchbox")).toBeNull());
  });
});
