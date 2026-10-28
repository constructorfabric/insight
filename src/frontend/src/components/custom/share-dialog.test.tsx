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
vi.mock("@/components/ui/sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}));
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import { CustomApiError, type GrantTarget } from "@/api/custom-client";
import { toast } from "@/components/ui/sonner";

import { ShareDialog } from "./share-dialog";

const { OWNER, BOB, CAROL } = people.ids;

const granted = (grantee: string, level: "viewer" | "editor") => ({
  grantee,
  level,
  granted_by: OWNER,
  created_at: "2026-09-01T09:00:00Z",
});

const REFUSALS: Record<
  GrantTarget["kind"],
  { reason: string; sentence: string }
> = {
  dashboards: {
    reason:
      "only the owner or an editor granted this dashboard directly or through everyone can share it",
    sentence: "Editors through a folder can't share this dashboard.",
  },
  folders: {
    reason: "only the owner or an editor can change this folder",
    sentence: "Only the owner or an editor can change this folder.",
  },
};

const TARGETS: [string, GrantTarget][] = [
  ["a dashboard", { kind: "dashboards", id: "dash-1" }],
  ["a folder", { kind: "folders", id: "folder-1" }],
];

function Opened({
  target,
  onClose,
}: {
  target: GrantTarget;
  onClose: () => void;
}) {
  const [open, setOpen] = useState(true);

  return (
    <ShareDialog
      target={target}
      title="Delivery"
      ownerPersonId={OWNER}
      open={open}
      onClose={() => {
        setOpen(false);
        onClose();
      }}
    />
  );
}

function renderDialog(target: GrantTarget, onClose = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <Opened target={target} onClose={onClose} />
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
      new CustomApiError(400, {
        detail: "Invalid argument",
        context: {
          field_violations: [
            { field: "grantee", description: "no person has the id" },
          ],
        },
      })
    );
    const onClose = vi.fn();
    const user = renderDialog(target, onClose);
    await within(sharedWith()).findByText("Carol Chen");
    await user.type(screen.getByRole("searchbox"), "an");

    await user.click(screen.getByRole("button", { name: "Dave Liu" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "no person has the id"
    );
    expect(onClose).not.toHaveBeenCalled();
  });

  it.each([
    ["sharing with someone", "put"],
    ["withdrawing a grant", "delete"],
  ] as const)(
    "closes with the service's sentence, once, when %s is refused",
    async (_case, change) => {
      const refused = new CustomApiError(403, {
        detail: "You do not have permission to perform this operation",
        context: { reason: REFUSALS[target.kind].reason },
      });
      vi.mocked(customClient.putGrant).mockRejectedValue(refused);
      vi.mocked(customClient.deleteGrant).mockRejectedValue(refused);
      const onClose = vi.fn();
      const user = renderDialog(target, onClose);
      await within(sharedWith()).findByText("Carol Chen");

      if (change === "put") {
        await user.type(screen.getByRole("searchbox"), "an");
        await user.click(screen.getByRole("button", { name: "Dave Liu" }));
      } else {
        await user.click(
          screen.getByRole("button", { name: "Stop sharing with Carol Chen" })
        );
      }

      await waitFor(() => expect(onClose, `case: ${_case}`).toHaveBeenCalled());
      expect(toast.error).toHaveBeenCalledOnce();
      expect(toast.error).toHaveBeenCalledWith(REFUSALS[target.kind].sentence);
      expect(screen.queryByRole("alert")).toBeNull();
    }
  );

  it("closes with the service's word when who it is shared with may not be read", async () => {
    vi.mocked(customClient.fetchGrants).mockRejectedValue(
      new CustomApiError(403, {
        detail: "You do not have permission to perform this operation",
        context: { reason: "a reason this portal does not know" },
      })
    );
    const onClose = vi.fn();
    renderDialog(target, onClose);

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(toast.error).toHaveBeenCalledWith(
      "You do not have permission to perform this operation"
    );
  });

  it.each([
    ["withdraws", "Stop sharing with Everyone", undefined],
    ["lowers", "Level for Everyone", "Viewer"],
  ])(
    "closes quietly when the caller %s the grant they shared through",
    async (_case, control, option) => {
      vi.mocked(customClient.fetchGrants).mockResolvedValueOnce([
        granted(CAROL, "viewer"),
        granted("everyone", "editor"),
      ]);
      vi.mocked(customClient.fetchGrants).mockRejectedValue(
        new CustomApiError(403, {
          detail: "You do not have permission to perform this operation",
          context: { reason: REFUSALS[target.kind].reason },
        })
      );
      const onClose = vi.fn();
      const user = renderDialog(target, onClose);
      await within(sharedWith()).findByText("Carol Chen");

      if (option) {
        await pick(user, control, option);
      } else {
        await user.click(screen.getByRole("button", { name: control }));
      }

      await waitFor(() => expect(onClose, `case: ${_case}`).toHaveBeenCalled());
      expect(toast.error).not.toHaveBeenCalled();
      expect(screen.queryByRole("alert")).toBeNull();
    }
  );
});
