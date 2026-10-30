vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    createDefinition: vi.fn(),
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
const assistant = vi.hoisted(() => ({ available: false }));
vi.mock("@/lib/portal/use-assistant-available", () => ({
  useAssistantAvailable: () => assistant.available,
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
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import { toast } from "@/components/ui/sonner";
import { useAssistantDraft } from "@/lib/custom/assistant-draft";
import { portalRouter } from "@/test/portal-router";

import { CreateDashboardDialog } from "./create-dashboard-dialog";

const { BOB, CAROL } = people.ids;
const NEW_ID = "dash-new";
const NEW_TARGET = { kind: "dashboards", id: NEW_ID };

function DraftProbe() {
  const draft = useAssistantDraft();

  return <output aria-label="Assistant draft">{draft?.text ?? ""}</output>;
}

function renderDialog(onClose = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <CreateDashboardDialog open onClose={onClose} />
      <DraftProbe />
    </QueryClientProvider>
  );

  return { user: userEvent.setup(), onClose };
}

const nameField = () => screen.getByRole("textbox", { name: "Name" });
const createButton = () => screen.getByRole("button", { name: /Create/ });

beforeEach(() => {
  vi.clearAllMocks();
  assistant.available = false;
  portalRouter.reset("/portal/custom");
  vi.mocked(customClient.createDefinition).mockResolvedValue(NEW_ID);
  vi.mocked(customClient.putGrant).mockResolvedValue(undefined);
});

describe("<CreateDashboardDialog>", () => {
  it("opens on the name, private, with Create held until a name is typed", async () => {
    const { user } = renderDialog();

    expect(screen.getByRole("dialog", { name: "New dashboard" })).toBeVisible();
    await waitFor(() => expect(nameField()).toHaveFocus());
    expect(screen.getByRole("radio", { name: /^Private/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /^Shared with org/ })).not.toBeChecked();
    expect(createButton()).toBeDisabled();

    await user.type(nameField(), "   ");
    expect(createButton()).toBeDisabled();

    await user.type(nameField(), "Delivery");
    expect(createButton()).toBeEnabled();
  });

  it("creates an empty private dashboard, shares it with no one and opens it", async () => {
    const { user, onClose } = renderDialog();

    await user.type(nameField(), "  Delivery ");
    await user.click(createButton());

    await waitFor(() =>
      expect(portalRouter.pathname).toBe(`/portal/custom/${NEW_ID}`)
    );
    expect(customClient.createDefinition).toHaveBeenCalledWith(
      "dashboards",
      "Delivery",
      { title: "Delivery", items: [] }
    );
    expect(customClient.putGrant).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("creates on Enter in the name field", async () => {
    const { user } = renderDialog();

    await user.type(nameField(), "Delivery{Enter}");

    await waitFor(() =>
      expect(customClient.createDefinition).toHaveBeenCalledTimes(1)
    );
  });

  it("shares with each named person at the viewer level once created", async () => {
    const { user } = renderDialog();
    await user.type(nameField(), "Delivery");

    await user.click(screen.getByRole("radio", { name: /^Shared with people/ }));
    expect(createButton()).toBeDisabled();

    await user.type(screen.getByRole("searchbox"), "an");
    await user.click(screen.getByRole("button", { name: "Bob Park" }));
    await user.click(screen.getByRole("button", { name: "Carol Chen" }));
    expect(
      within(screen.getByRole("list", { name: "Named people" })).getAllByRole(
        "listitem"
      )
    ).toHaveLength(2);

    await user.click(createButton());

    await waitFor(() =>
      expect(portalRouter.pathname).toBe(`/portal/custom/${NEW_ID}`)
    );
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

  it("drops a named person before creating", async () => {
    const { user } = renderDialog();
    await user.type(nameField(), "Delivery");
    await user.click(screen.getByRole("radio", { name: /^Shared with people/ }));
    await user.type(screen.getByRole("searchbox"), "an");
    await user.click(screen.getByRole("button", { name: "Bob Park" }));
    await user.click(screen.getByRole("button", { name: "Carol Chen" }));

    await user.click(screen.getByRole("button", { name: "Remove Bob Park" }));
    await user.click(createButton());

    await waitFor(() =>
      expect(portalRouter.pathname).toBe(`/portal/custom/${NEW_ID}`)
    );
    expect(customClient.putGrant).toHaveBeenCalledTimes(1);
    expect(customClient.putGrant).toHaveBeenCalledWith(
      NEW_TARGET,
      CAROL,
      "viewer"
    );
  });

  it("shares with everyone at the viewer level when chosen", async () => {
    const { user } = renderDialog();
    await user.type(nameField(), "Delivery");

    await user.click(screen.getByRole("radio", { name: /^Shared with org/ }));
    await user.click(createButton());

    await waitFor(() =>
      expect(customClient.putGrant).toHaveBeenCalledWith(
        NEW_TARGET,
        "everyone",
        "viewer"
      )
    );
  });

  it("holds the dialog while creating", async () => {
    vi.mocked(customClient.createDefinition).mockReturnValue(
      new Promise(() => {})
    );
    const { user } = renderDialog();
    await user.type(nameField(), "Delivery");

    await user.click(createButton());

    expect(
      await screen.findByRole("status", { name: "Loading" })
    ).toBeVisible();
    expect(createButton()).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  });

  it("says what the service said when creating is refused, keeping the name", async () => {
    vi.mocked(customClient.createDefinition).mockRejectedValue(
      new CustomApiError(400, {
        detail: "Invalid argument",
        context: {
          field_violations: [
            { field: "name", description: "name must not be blank" },
          ],
        },
      })
    );
    const { user, onClose } = renderDialog();
    await user.type(nameField(), "Delivery");

    await user.click(createButton());

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "name must not be blank"
    );
    expect(nameField()).toHaveValue("Delivery");
    expect(portalRouter.navigations).toHaveLength(0);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("opens the new dashboard and says so when a share is refused after creating", async () => {
    vi.mocked(customClient.putGrant).mockRejectedValue(
      new CustomApiError(403, {
        detail: "Forbidden",
        context: { reason: "admin role required for this operation" },
      })
    );
    const { user } = renderDialog();
    await user.type(nameField(), "Delivery");
    await user.click(screen.getByRole("radio", { name: /^Shared with org/ }));

    await user.click(createButton());

    await waitFor(() =>
      expect(portalRouter.pathname).toBe(`/portal/custom/${NEW_ID}`)
    );
    expect(toast.error).toHaveBeenCalledWith(
      "Only an administrator can do this."
    );
  });

  it("offers no start choice while the Assistant is unavailable", () => {
    renderDialog();

    expect(screen.queryByRole("radio", { name: "Empty dashboard" })).toBeNull();
    expect(
      screen.queryByRole("radio", { name: "Describe it to the Assistant" })
    ).toBeNull();
  });

  it.each([
    ["from Dashboards, staying there", "/portal/custom", 0],
    ["from Home, moving to Dashboards", "/portal", 1],
  ])(
    "hands the name to the Assistant %s, creating nothing",
    async (_where, pathname, navigations) => {
      assistant.available = true;
      portalRouter.reset(pathname);
      const { user, onClose } = renderDialog();
      await user.type(nameField(), "Delivery");
      expect(
        screen.getByRole("radio", { name: "Empty dashboard" })
      ).toBeChecked();

      await user.click(
        screen.getByRole("radio", { name: "Describe it to the Assistant" })
      );
      await user.click(createButton());

      expect(
        screen.getByLabelText("Assistant draft")
      ).toHaveTextContent('Build a dashboard named "Delivery" that shows:');
      expect(customClient.createDefinition).not.toHaveBeenCalled();
      expect(onClose).toHaveBeenCalled();
      expect(portalRouter.navigations).toHaveLength(navigations);
      expect(portalRouter.pathname).toBe("/portal/custom");
    }
  );
});
