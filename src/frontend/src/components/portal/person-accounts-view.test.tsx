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
    data: undefined as { items: unknown[]; truncated?: boolean } | undefined,
    isFetching: false,
    isError: false,
  },
}));
vi.mock("@/queries/identity-resolution", () => ({
  usePersonAccounts: () => hooks.accounts,
  usePersonSearch: () => hooks.search,
  // The window's own behaviour belongs to account-detail.test.
  useAccountBinding: () => ({
    data: undefined,
    isLoading: true,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
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
  portalRouter.reset();
  portalRouter.set({ zone: "manage", item: "identities", mode: "people" });
});

describe("PersonAccountsView", () => {
  it("asks for a person before showing anything", () => {
    render(<PersonAccountsView />);

    expect(screen.getByText(/nobody chosen yet/i)).toBeInTheDocument();
  });

  it("puts the chosen person in the URL, so the view is a link", async () => {
    hooks.search.data = {
      items: [{ person_id: ANN, display_name: "Ann Lee", email: "ann@example.com" }],
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
