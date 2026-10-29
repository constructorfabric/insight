vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    createFolder: vi.fn(),
    putGrant: vi.fn(),
  };
});
const people = vi.hoisted(() => {
  const BOB = "01900000-0000-7000-8000-0000000000b0";
  const CAROL = "01900000-0000-7000-8000-0000000000c0";
  const names: Record<string, string> = {
    [BOB]: "Bob Park",
    [CAROL]: "Carol Chen",
  };

  return { ids: { BOB, CAROL }, names };
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
const admin = vi.hoisted(() => ({ isAdmin: true }));
vi.mock("@/queries/identity-me", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-me")>()),
  useIsAdmin: () => ({ isAdmin: admin.isAdmin, isPending: false }),
}));
vi.mock("@/components/ui/sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}));
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import { toast } from "@/components/ui/sonner";
import { foldersQuery } from "@/queries/custom";

import { CreateFolderDialog } from "./create-folder-dialog";

const { BOB, CAROL } = people.ids;
const NEW_TARGET = { kind: "folders", id: "f-new" };

function renderDialog() {
  const onClose = vi.fn();
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const invalidated = vi.spyOn(queryClient, "invalidateQueries");
  const shown = (open: boolean) => (
    <QueryClientProvider client={queryClient}>
      <CreateFolderDialog open={open} onClose={onClose} />
    </QueryClientProvider>
  );
  const { rerender } = render(shown(true));

  return {
    user: userEvent.setup(),
    onClose,
    invalidated,
    reopen: () => {
      rerender(shown(false));
      rerender(shown(true));
    },
  };
}

const nameField = () => screen.getByRole("textbox", { name: "Name" });
const createButton = () => screen.getByRole("button", { name: /Create/ });

beforeEach(() => {
  vi.clearAllMocks();
  admin.isAdmin = true;
  vi.mocked(customClient.createFolder).mockResolvedValue({
    id: "f-new",
    name: "Hiring",
  });
  vi.mocked(customClient.putGrant).mockResolvedValue(undefined);
});

describe("<CreateFolderDialog>", () => {
  it("opens on the name, private, with Create held until a name is typed", async () => {
    const { user } = renderDialog();

    expect(screen.getByRole("dialog", { name: "New folder" })).toBeVisible();
    await waitFor(() => expect(nameField()).toHaveFocus());
    expect(screen.getByRole("radio", { name: /^Private/ })).toBeChecked();
    expect(createButton()).toBeDisabled();

    await user.type(nameField(), "   ");
    expect(createButton()).toBeDisabled();

    await user.type(nameField(), "Hiring");
    expect(createButton()).toBeEnabled();
  });

  it("says that sharing a folder shares the dashboards in it", () => {
    renderDialog();

    expect(
      screen.getByRole("radio", { name: /^Shared with org/ })
    ).toHaveAccessibleName(/Everyone can view its dashboards/);
    expect(
      screen.getByRole("radio", { name: /^Shared with people/ })
    ).toHaveAccessibleName(/People you pick can view its dashboards/);
  });

  it("creates a private folder on Enter, shares it with no one, closes and refreshes the rail", async () => {
    const { user, onClose, invalidated } = renderDialog();

    await user.type(nameField(), "  Hiring {Enter}");

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(customClient.createFolder).toHaveBeenCalledWith("Hiring");
    expect(customClient.putGrant).not.toHaveBeenCalled();
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: foldersQuery().queryKey,
    });
  });

  it("shares with everyone at the viewer level when chosen", async () => {
    const { user, onClose } = renderDialog();
    await user.type(nameField(), "Hiring");

    await user.click(screen.getByRole("radio", { name: /^Shared with org/ }));
    await user.click(createButton());

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(customClient.putGrant).toHaveBeenCalledWith(
      NEW_TARGET,
      "everyone",
      "viewer"
    );
  });

  it("shares with each named person once one is picked", async () => {
    const { user, onClose } = renderDialog();
    await user.type(nameField(), "Hiring");

    await user.click(
      screen.getByRole("radio", { name: /^Shared with people/ })
    );
    expect(createButton()).toBeDisabled();

    await user.type(screen.getByRole("searchbox"), "an");
    await user.click(screen.getByRole("button", { name: "Bob Park" }));
    await user.click(screen.getByRole("button", { name: "Carol Chen" }));
    await user.click(createButton());

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(customClient.putGrant).toHaveBeenCalledWith(
      NEW_TARGET,
      BOB,
      "viewer"
    );
    expect(customClient.putGrant).toHaveBeenCalledWith(
      NEW_TARGET,
      CAROL,
      "viewer"
    );
  });

  it("says why a name was refused, keeping the dialog and the name", async () => {
    vi.mocked(customClient.createFolder).mockRejectedValue(
      new CustomApiError(409, {
        detail: "a folder named `hiring` already exists",
      })
    );
    const { user, onClose, invalidated } = renderDialog();
    await user.type(nameField(), "hiring");

    await user.click(createButton());

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "a folder named `hiring` already exists"
    );
    expect(nameField()).toHaveValue("hiring");
    expect(onClose).not.toHaveBeenCalled();
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: foldersQuery().queryKey,
    });
  });

  it("keeps the folder and says so when a share is refused after creating", async () => {
    vi.mocked(customClient.putGrant).mockRejectedValue(
      new CustomApiError(403, {
        detail: "Forbidden",
        context: { reason: "admin role required for this operation" },
      })
    );
    const { user, onClose } = renderDialog();
    await user.type(nameField(), "Hiring");
    await user.click(screen.getByRole("radio", { name: /^Shared with org/ }));

    await user.click(createButton());

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(toast.error).toHaveBeenCalledWith(
      "Only an administrator can do this."
    );
  });

  it("offers a non-administrator no sharing and creates the folder private", async () => {
    admin.isAdmin = false;
    const { user, onClose } = renderDialog();

    expect(screen.queryByRole("group", { name: "Visibility" })).toBeNull();
    await user.type(nameField(), "Hiring{Enter}");

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(customClient.createFolder).toHaveBeenCalledWith("Hiring");
    expect(customClient.putGrant).not.toHaveBeenCalled();
  });

  it("holds the dialog while creating", async () => {
    vi.mocked(customClient.createFolder).mockReturnValue(new Promise(() => {}));
    const { user, onClose } = renderDialog();
    await user.type(nameField(), "Hiring");

    await user.click(createButton());

    expect(
      await screen.findByRole("status", { name: "Loading" })
    ).toBeVisible();
    expect(createButton()).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("starts clean when reopened after a refused name", async () => {
    vi.mocked(customClient.createFolder).mockRejectedValue(
      new CustomApiError(409, {
        detail: "a folder named `hiring` already exists",
      })
    );
    const { user, onClose, reopen } = renderDialog();
    await user.type(nameField(), "hiring");
    await user.click(
      screen.getByRole("radio", { name: /^Shared with people/ })
    );
    await user.type(screen.getByRole("searchbox"), "an");
    await user.click(screen.getByRole("button", { name: "Bob Park" }));
    await user.click(createButton());
    expect(await screen.findByRole("alert")).toBeVisible();
    expect(nameField()).toHaveAccessibleDescription(
      "a folder named `hiring` already exists"
    );

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onClose).toHaveBeenCalled();
    reopen();

    expect(nameField()).toHaveValue("");
    expect(screen.getByRole("radio", { name: /^Private/ })).toBeChecked();
    expect(screen.queryByRole("alert")).toBeNull();
    await user.click(
      screen.getByRole("radio", { name: /^Shared with people/ })
    );
    expect(screen.queryByRole("list", { name: "Named people" })).toBeNull();
  });
});
