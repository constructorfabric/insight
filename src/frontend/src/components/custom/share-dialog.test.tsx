vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchGrants: vi.fn(),
    putGrant: vi.fn(),
    deleteGrant: vi.fn(),
  };
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

  return { ids: { OWNER, BOB, CAROL, DAVE }, names };
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
import { CustomApiError, type GrantTarget } from "@/api/custom-client";

import { ShareDialog } from "./share-dialog";

const { OWNER, BOB, CAROL } = people.ids;

const granted = (grantee: string, level: "viewer" | "editor") => ({
  grantee,
  level,
  granted_by: OWNER,
  created_at: "2026-09-01T09:00:00Z",
});

const TARGETS: [string, GrantTarget][] = [
  ["a dashboard", { kind: "dashboards", id: "dash-1" }],
  ["a folder", { kind: "folders", id: "folder-1" }],
];

function renderDialog(target: GrantTarget) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ShareDialog
        target={target}
        title="Delivery"
        ownerPersonId={OWNER}
        open
        onClose={vi.fn()}
      />
    </QueryClientProvider>
  );

  return userEvent.setup();
}

const sharedWith = () => screen.getByRole("region", { name: "Shared with" });

async function pick(
  user: ReturnType<typeof userEvent.setup>,
  label: string,
  option: string
) {
  await user.click(screen.getByRole("combobox", { name: label }));
  await user.click(await screen.findByRole("option", { name: option }));
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(customClient.fetchGrants).mockResolvedValue([
    granted(CAROL, "viewer"),
    granted("everyone", "viewer"),
  ]);
  vi.mocked(customClient.putGrant).mockResolvedValue(undefined);
  vi.mocked(customClient.deleteGrant).mockResolvedValue(undefined);
});

describe.each(TARGETS)("<ShareDialog> for %s", (_label, target) => {
  it("lists Everyone first, then each person by name, with their level", async () => {
    renderDialog(target);

    await within(sharedWith()).findByText("Carol Chen");
    const rows = within(sharedWith()).getAllByRole("listitem");
    expect(rows.map((row) => row.textContent)).toEqual([
      expect.stringContaining("Everyone"),
      expect.stringContaining("Carol Chen"),
    ]);
    expect(
      within(sharedWith()).getByRole("combobox", {
        name: "Level for Carol Chen",
      })
    ).toHaveTextContent("Viewer");
    expect(customClient.fetchGrants).toHaveBeenCalledWith(target);
  });

  it("shares with a picked person at the chosen level, offering neither the owner nor who already has it", async () => {
    vi.mocked(customClient.fetchGrants).mockResolvedValue([
      granted(CAROL, "viewer"),
    ]);
    const user = renderDialog(target);
    await within(sharedWith()).findByText("Carol Chen");

    await pick(user, "Level", "Editor");
    await user.type(screen.getByRole("searchbox"), "an");

    expect(screen.queryByRole("button", { name: "Ann Owner" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Carol Chen" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Bob Park" }));

    expect(customClient.putGrant).toHaveBeenCalledWith(target, BOB, "editor");
  });

  it("shares with everyone at the chosen level, once", async () => {
    vi.mocked(customClient.fetchGrants).mockResolvedValue([]);
    const user = renderDialog(target);
    await within(sharedWith()).findByText("No one");

    await user.click(screen.getByRole("button", { name: "Everyone" }));

    expect(customClient.putGrant).toHaveBeenCalledWith(
      target,
      "everyone",
      "viewer"
    );
  });

  it("offers no second Everyone grant once everyone has it", async () => {
    renderDialog(target);
    await within(sharedWith()).findByText("Carol Chen");

    expect(screen.getByRole("button", { name: "Everyone" })).toBeDisabled();
  });

  it("changes a grant's level by granting it again", async () => {
    const user = renderDialog(target);
    await within(sharedWith()).findByText("Carol Chen");

    await pick(user, "Level for Carol Chen", "Editor");

    expect(customClient.putGrant).toHaveBeenCalledWith(target, CAROL, "editor");
  });

  it.each([
    ["Carol Chen", CAROL],
    ["Everyone", "everyone"],
  ])("withdraws the grant to %s", async (name, grantee) => {
    const user = renderDialog(target);
    await within(sharedWith()).findByText("Carol Chen");

    await user.click(
      screen.getByRole("button", { name: `Stop sharing with ${name}` })
    );

    expect(customClient.deleteGrant).toHaveBeenCalledWith(target, grantee);
    await waitFor(() =>
      expect(
        vi.mocked(customClient.fetchGrants).mock.calls.length
      ).toBeGreaterThan(1)
    );
  });

  it("says what the service said when a grant is refused", async () => {
    vi.mocked(customClient.putGrant).mockRejectedValue(
      new CustomApiError(403, {
        detail: "only the owner or an editor can share this",
      })
    );
    const user = renderDialog(target);
    await within(sharedWith()).findByText("Carol Chen");
    await user.type(screen.getByRole("searchbox"), "an");

    await user.click(screen.getByRole("button", { name: "Dave Liu" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "only the owner or an editor can share this"
    );
  });
});
