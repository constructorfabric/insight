// @vitest-environment jsdom
/**
 * The console's second mode. What matters: an operator arrives holding a name
 * and reaches accounts the queue never shows, because a settled binding is not
 * a problem — until someone asks about it; who decided each binding is on the
 * row, since undoing automation is routine and overruling a colleague is not;
 * and the person rides in the URL, so the view is shareable like the rest.
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

const hooks = vi.hoisted(() => ({
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
  verb: {
    mutate: vi.fn(),
    reset: vi.fn(),
    isPending: false,
    isError: false,
    error: null as unknown,
  },
  binding: {
    data: undefined as unknown,
    isLoading: true,
    isError: false,
    error: null as unknown,
    refetch: vi.fn(),
  },
  accountSearch: {
    data: undefined as { pages: { items: unknown[] }[] } | undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isPlaceholderData: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  },
}));
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  usePersonAccounts: () => hooks.accounts,
  usePersonList: () => hooks.search,
  useAccountList: () => hooks.accountSearch,
  // The window's own behaviour belongs to account-detail.test; a case that needs
  // the verbs on screen sets a binding for itself.
  useAccountBinding: () => hooks.binding,
  // The verbs themselves belong to account-actions.test; stubbed so the window
  // can render them without this suite standing up a query client.
  useBindAccount: () => hooks.verb,
  useMergePersons: () => hooks.verb,
  useDetachAccount: () => hooks.verb,
  useExcludeAccount: () => hooks.verb,
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

beforeEach(() => {
  hooks.accounts.data = undefined;
  hooks.accounts.isLoading = false;
  hooks.accounts.isError = false;
  hooks.accounts.refetch.mockClear();
  hooks.search.data = undefined;
  hooks.binding.data = undefined;
  hooks.binding.isLoading = true;
  hooks.accountSearch.data = undefined;
  hooks.accountSearch.hasNextPage = false;
  hooks.accountSearch.isPlaceholderData = false;
  hooks.accountSearch.fetchNextPage.mockClear();
  portalRouter.reset();
  portalRouter.set({ zone: "manage", item: "identities", mode: "person" });
});

describe("PersonAccountsView", () => {
  // With nobody chosen the mode IS the roster: an operator reviewing
  // identities needs to see who exists, not guess a name to type.
  it("lists the roster before anyone is chosen", () => {
    hooks.search.data = {
      pages: [
        { items: [{ person_id: ANN, display_name: "Ann Lee", email: "ann@example.com" }] },
      ],
    };
    render(<PersonAccountsView />);

    expect(screen.getByText("Ann Lee")).toBeInTheDocument();
  });

  // Choosing a person swaps the roster for their accounts and clears the terms
  // that found them, so nothing on screen leads back — an operator comparing
  // two people would be retyping, or editing the URL.
  it("offers the way back to the roster once a person is chosen", async () => {
    // A value the window cannot open, so the control is not under a modal: a
    // live account key opens the case dialog, whose own Close is the way out of
    // that. What this pins is that returning to the roster leaves no account
    // selection behind, not that it can dismiss an open one.
    portalRouter.set({ person: ANN, acct: "malformed" });
    hooks.accounts.data = { person_id: ANN, accounts: [] };
    render(<PersonAccountsView />);

    await userEvent.click(screen.getByRole("button", { name: /back to the roster/i }));

    expect(portalRouter.search.person).toBeUndefined();
    expect(portalRouter.search.acct).toBeUndefined();
  });

  it("shows no way back while the roster itself is on screen", () => {
    render(<PersonAccountsView />);

    expect(
      screen.queryByRole("button", { name: /back to the roster/i }),
    ).not.toBeInTheDocument();
  });

  it("puts the chosen person in the URL, so the view is a link", async () => {
    hooks.search.data = {
      pages: [
        { items: [{ person_id: ANN, display_name: "Ann Lee", email: "ann@example.com" }] },
      ],
    };
    render(<PersonAccountsView />);

    await userEvent.type(screen.getByRole("searchbox"), "ann");
    await userEvent.click(screen.getByRole("button", { name: /ann lee/i }));

    expect(portalRouter.search.person).toBe(ANN);
  });

  it("lists every account bound to the person, saying who decided each", () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.data = {
      person_id: ANN,
      accounts: [
        entry(),
        entry({
          source: "gitlab",
          account_id: "gl-alt",
          email: null,
          username: "alee",
          bound_by_operator: true,
        }),
      ],
    };
    render(<PersonAccountsView />);

    expect(screen.getByText("ann@example.com")).toBeInTheDocument();
    expect(screen.getByText(/github · gh-main/)).toBeInTheDocument();
    expect(screen.getByText(/bound automatically/i)).toBeInTheDocument();
    expect(screen.getByText(/decided by an operator/i)).toBeInTheDocument();
  });

  // The whole point of the mode: these accounts are settled, so the queue
  // never shows them — and the verbs are still available for them.
  it("opens a settled account in the same case window", async () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(<PersonAccountsView />);

    await userEvent.click(screen.getByRole("button", { name: /^open$/i }));

    expect(portalRouter.search.acct).toContain("gh-main");
    expect(within(screen.getByRole("dialog")).getByText(/github · gh-main/)).toBeInTheDocument();
  });

  // Inside a person the people search is the wrong question — they are already
  // open, and the back link is the way out. What is useful here is finding an
  // ACCOUNT, to bind it to them.
  it("searches accounts, not people, once a person is open", () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(<PersonAccountsView />);

    const fields = screen.getAllByRole("searchbox");
    expect(fields).toHaveLength(1);
    expect(fields[0]).toHaveAccessibleName(/find an account/i);
  });

  // The whole fold would bury the handful of accounts the person actually
  // holds, which is what the reader opened them for.
  it("lists no account until the field is asked something", () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    hooks.accountSearch.data = {
      pages: [
        {
          items: [
            {
              source: "zoom",
              source_id: "01900000-0000-7000-8000-00000000aa03",
              account_id: "zm-9",
              email: "someone.else@example.com",
              username: null,
              display_name: null,
              person: null,
              bound_by_operator: false,
            },
          ],
        },
      ],
    };
    render(<PersonAccountsView />);

    // Whatever the hook holds, a blank field asked nothing and shows nothing.
    expect(screen.queryByText("someone.else@example.com")).not.toBeInTheDocument();
    expect(screen.getByText("ann@example.com")).toBeInTheDocument();
  });

  // One window per surface: `?acct=` opens by itself, so a second one would
  // open beside the first on the same link.
  it("opens a found account in the same window the person's own rows use", async () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(<PersonAccountsView />);

    await userEvent.click(screen.getByRole("button", { name: /^open$/i }));

    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });

  // Every account here is already this person's, so a candidate list saying so
  // invites re-asserting the binding — the confirm act, which belongs in the
  // queue. The holder is named in its own section instead.
  it("opens one of the person's accounts with no candidates to confirm", async () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    // A real binding, or the window body is a spinner and this proves nothing.
    hooks.binding.data = {
      source: "github",
      source_id: "01900000-0000-7000-8000-00000000aa01",
      account_id: "gh-main",
      person_id: ANN,
      history: [],
    };
    hooks.binding.isLoading = false;
    render(<PersonAccountsView />);

    await userEvent.click(screen.getByRole("button", { name: /^open$/i }));

    const dialog = screen.getByRole("dialog");
    // The verbs are on screen — so an absent candidate list means absent, not
    // "still loading".
    expect(
      within(dialog).getByRole("button", { name: /exclude \(bot/i }),
    ).toBeInTheDocument();
    expect(within(dialog).queryByText(/candidates/i)).not.toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: /^confirm$/i }),
    ).not.toBeInTheDocument();
  });

  it("states an empty result rather than an empty card", () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.data = { person_id: ANN, accounts: [] };
    render(<PersonAccountsView />);

    expect(screen.getByText(/no account is bound to this person/i)).toBeInTheDocument();
  });

  it("offers a retry when the read fails", async () => {
    portalRouter.set({ person: ANN });
    hooks.accounts.isError = true;
    render(<PersonAccountsView />);

    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(hooks.accounts.refetch).toHaveBeenCalled();
  });
});
