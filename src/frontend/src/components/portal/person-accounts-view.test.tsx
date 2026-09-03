// @vitest-environment jsdom
/**
 * The console's second mode. What matters: an operator arrives holding a name
 * and reaches accounts the queue never shows, because a settled binding is not
 * a problem — until someone asks about it; the roster STAYS on screen while a
 * person is open, exactly as the account listing does behind its own window;
 * and the person rides in the URL, so the view is shareable like the rest.
 *
 * What the window itself does belongs to `person-dialog.test.tsx`.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { PersonAccountEntry } from "@/api/identity-client";

vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

const hooks = vi.hoisted(() => {
  const verb = () => ({
    mutate: vi.fn(),
    reset: vi.fn(),
    isPending: false,
    isError: false,
    error: null as unknown,
  });
  return {
  accounts: {
    data: undefined as
      | { person_id: string; accounts: PersonAccountEntry[] }
      | undefined,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  },
  search: {
    data: undefined as { pages: { items: unknown[] }[] } | undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  },
  bind: verb(),
  detach: verb(),
  exclude: verb(),
  accountSearch: {
    data: undefined as { pages: { items: unknown[] }[] } | undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isPlaceholderData: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  },
  };
});
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  usePersonAccounts: () => hooks.accounts,
  usePersonList: () => hooks.search,
  useAccountList: () => hooks.accountSearch,
  // The verbs belong to person-dialog.test; stubbed so the window can render
  // them without this suite standing up a query client.
  useBindAccount: () => hooks.bind,
  useDetachAccount: () => hooks.detach,
  useExcludeAccount: () => hooks.exclude,
}));

import { portalRouter } from "@/test/portal-router";

import { PersonAccountsView } from "./person-accounts-view";

const ANN = "01900000-0000-7000-8000-0000000000a0";

function entry(over: Partial<PersonAccountEntry> = {}): PersonAccountEntry {
  return {
    source: "github",
    source_id: "01900000-0000-7000-8000-00000000aa01",
    account_id: "gh-main",
    email: "ann@example.com",
    username: null,
    bound_by_operator: false,
    ...over,
  };
}

function roster() {
  hooks.search.data = {
    pages: [
      {
        items: [
          { person_id: ANN, display_name: "Ann Lee", email: "ann@example.com" },
        ],
      },
    ],
  };
}

beforeEach(() => {
  hooks.accounts.data = undefined;
  hooks.accounts.isLoading = false;
  hooks.accounts.isError = false;
  hooks.accounts.refetch.mockClear();
  hooks.search.data = undefined;
  hooks.search.isFetching = false;
  hooks.search.isFetchingNextPage = false;
  hooks.search.isError = false;
  hooks.search.hasNextPage = false;
  hooks.search.fetchNextPage.mockClear();
  hooks.accountSearch.data = undefined;
  hooks.accountSearch.isFetching = false;
  hooks.accountSearch.isFetchingNextPage = false;
  hooks.accountSearch.isError = false;
  hooks.accountSearch.hasNextPage = false;
  hooks.accountSearch.isPlaceholderData = false;
  hooks.accountSearch.fetchNextPage.mockClear();
  for (const verb of [hooks.bind, hooks.detach, hooks.exclude]) {
    verb.mutate.mockReset();
    verb.reset.mockReset();
    verb.isPending = false;
    verb.isError = false;
    verb.error = null;
  }
  portalRouter.reset();
  portalRouter.set({ zone: "manage", item: "identities", mode: "person" });
});

describe("PersonAccountsView", () => {
  // With nobody chosen the mode IS the roster: an operator reviewing
  // identities needs to see who exists, not guess a name to type.
  it("lists the roster before anyone is chosen", () => {
    roster();
    render(<PersonAccountsView />);

    expect(screen.getByText("Ann Lee")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("puts the chosen person in the URL, so the view is a link", async () => {
    roster();
    render(<PersonAccountsView />);

    await userEvent.type(screen.getByRole("searchbox"), "ann");
    await userEvent.click(screen.getByRole("button", { name: /ann lee/i }));

    expect(portalRouter.search.person).toBe(ANN);
  });

  // The point of the redesign: the roster does not go away. Both listings in
  // this console now behave the same — a window opens over what you were
  // reading, rather than replacing the screen and needing a way back drawn on
  // it.
  it("keeps the roster behind the window a person opens", () => {
    roster();
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(<PersonAccountsView />);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("Ann Lee")).toBeInTheDocument();
  });

  // Closing IS the way back, so no second control has to say so.
  it("clears the person from the URL when the window closes", async () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(<PersonAccountsView />);

    await userEvent.click(screen.getByRole("button", { name: /close/i }));

    expect(portalRouter.search.person).toBeUndefined();
  });

  // Two different questions, so two different fields — and never one field
  // pointed at both. The surface finds PEOPLE; the window over it finds
  // ACCOUNTS, to bind them to the person it is about.
  it("searches people on the surface and accounts inside the window", () => {
    roster();
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(<PersonAccountsView />);

    expect(
      within(screen.getByRole("dialog")).getByRole("searchbox", {
        name: /find an account/i,
      }),
    ).toBeInTheDocument();
    // By placeholder, not by role: the window marks the page behind it hidden
    // from assistive tech, which is exactly what an open dialog should do — the
    // roster is still mounted and still holds its own terms.
    expect(screen.getByPlaceholderText(/search people/i)).toBeInTheDocument();
  });
});
