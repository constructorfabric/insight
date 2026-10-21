vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchOrphanedDashboards: vi.fn(),
    transferDashboard: vi.fn(),
  };
});
vi.mock("@/api/identity-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/identity-client")>();
  return { ...actual, listRoleHolders: vi.fn() };
});
const people = vi.hoisted(() => {
  const LEAVER = "01900000-0000-7000-8000-0000000000e0";
  const BOB = "01900000-0000-7000-8000-0000000000b0";
  const DAVE = "01900000-0000-7000-8000-0000000000d0";
  const names: Record<string, string> = {
    [LEAVER]: "Erin Left",
    [BOB]: "Bob Park",
    [DAVE]: "Dave Liu",
  };

  return { ids: { LEAVER, BOB, DAVE }, names };
});
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  usePersonList: () => ({
    data: {
      pages: [
        {
          items: Object.entries(people.names).map(
            ([person_id, display_name]) => ({ person_id, display_name })
          ),
        },
      ],
    },
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

import { OrphanedDashboards } from "./orphaned-dashboards";

const { LEAVER, BOB } = people.ids;

const ORPHANED = {
  id: "dash-1",
  name: "Release notes",
  owner_person_id: LEAVER,
  updated_at: "2026-09-01T12:00:00.000Z",
};

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <OrphanedDashboards />
    </QueryClientProvider>
  );

  return userEvent.setup();
}

async function openTransfer(user: ReturnType<typeof userEvent.setup>) {
  await user.click(
    await screen.findByRole("button", { name: "Transfer Release notes" })
  );
  const dialog = await screen.findByRole("dialog");
  await user.type(await within(dialog).findByRole("searchbox"), "an");

  return dialog;
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(customClient.fetchOrphanedDashboards).mockResolvedValue({
    items: [ORPHANED],
    total: 1,
  });
  vi.mocked(customClient.transferDashboard).mockResolvedValue(undefined);
  vi.mocked(identityClient.listRoleHolders).mockResolvedValue({
    personIds: new Set([BOB]),
    complete: true,
  });
});

describe("<OrphanedDashboards>", () => {
  it("lists each orphaned dashboard with its owner and last change", async () => {
    renderPage();

    const row = (await screen.findByText("Release notes")).closest("tr");
    expect(row).not.toBeNull();
    expect(row).toHaveTextContent("Erin Left");
    expect(row).toHaveTextContent("1 Sep 2026");
  });

  it("says so when nothing is orphaned", async () => {
    vi.mocked(customClient.fetchOrphanedDashboards).mockResolvedValue({
      items: [],
      total: 0,
    });
    renderPage();

    expect(await screen.findByText("No orphaned dashboards")).toBeInTheDocument();
  });

  it("offers only current administrators as the new owner", async () => {
    const user = renderPage();
    const dialog = await openTransfer(user);

    expect(
      within(dialog).getByRole("button", { name: "Bob Park" })
    ).toBeInTheDocument();
    for (const name of ["Erin Left", "Dave Liu"]) {
      expect(
        within(dialog).queryByRole("button", { name }),
        `should not offer: ${name}`
      ).toBeNull();
    }
    expect(within(dialog).getByRole("button", { name: "Transfer" })).toBeDisabled();
  });

  it("transfers to the picked administrator and reads the list again", async () => {
    const user = renderPage();
    const dialog = await openTransfer(user);
    const reads = vi.mocked(customClient.fetchOrphanedDashboards).mock.calls
      .length;

    await user.click(within(dialog).getByRole("button", { name: "Bob Park" }));
    await user.click(within(dialog).getByRole("button", { name: "Transfer" }));

    expect(customClient.transferDashboard).toHaveBeenCalledWith("dash-1", BOB);
    await waitFor(() =>
      expect(
        vi.mocked(customClient.fetchOrphanedDashboards).mock.calls.length
      ).toBeGreaterThan(reads)
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the dialog open with what the service said when it refuses", async () => {
    vi.mocked(customClient.transferDashboard).mockRejectedValue(
      new CustomApiError(409, {
        detail: "Failed precondition",
        context: {
          violations: [
            {
              type: "owner_is_administrator",
              subject: "owner",
              description: "the dashboard's owner is still an administrator",
            },
          ],
        },
      })
    );
    const user = renderPage();
    const dialog = await openTransfer(user);

    await user.click(within(dialog).getByRole("button", { name: "Bob Park" }));
    await user.click(within(dialog).getByRole("button", { name: "Transfer" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "the dashboard's owner is still an administrator"
    );
  });
});
