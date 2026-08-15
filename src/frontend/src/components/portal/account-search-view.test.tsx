// @vitest-environment jsdom
/**
 * The account mode. What matters: an operator holding a handle or an address
 * learns whose it is — the question neither other mode can answer, since both
 * are entered through a person; unbound is stated as an answer rather than
 * left blank; and the account opens in the same case window, so the verbs are
 * one click from the search.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AccountMatch } from "@/api/identity-client";

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
    search: {
      data: undefined as { items: AccountMatch[]; truncated: boolean } | undefined,
      isFetching: false,
      isError: false,
    },
    binding: {
      data: undefined as unknown,
      isLoading: true,
      isError: false,
      error: null as unknown,
      refetch: vi.fn(),
    },
    verb,
  };
});
vi.mock("@/queries/identity-resolution", () => ({
  useAccountSearch: () => hooks.search,
  useAccountBinding: () => hooks.binding,
  useBindAccount: () => hooks.verb(),
  useMergePersons: () => hooks.verb(),
  useDetachAccount: () => hooks.verb(),
  useExcludeAccount: () => hooks.verb(),
  usePersonAccounts: () => ({
    data: undefined,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  }),
  usePersonSearch: () => ({ data: undefined, isFetching: false, isError: false }),
}));

import { portalRouter } from "@/test/portal-router";

import { AccountSearchView } from "./account-search-view";

function match(over: Partial<AccountMatch> = {}): AccountMatch {
  return {
    source: "github",
    source_id: "01900000-0000-7000-8000-00000000aa01",
    account_id: "gh-main",
    email: null,
    username: "octocat",
    display_name: null,
    person: {
      person_id: "01900000-0000-7000-8000-0000000000a0",
      display_name: "Ann Lee",
    },
    bound_by_operator: false,
    ...over,
  };
}

beforeEach(() => {
  hooks.search.data = undefined;
  hooks.search.isFetching = false;
  hooks.search.isError = false;
  hooks.binding.data = undefined;
  hooks.binding.isLoading = true;
  portalRouter.reset();
  portalRouter.set({ zone: "manage", item: "identities", mode: "accounts" });
});

describe("AccountSearchView", () => {
  // An empty surface reads as one that failed to load; the mode says what it
  // is for instead.
  it("says what to search for before anything is asked", () => {
    render(<AccountSearchView />);

    expect(screen.getByText(/nothing searched yet/i)).toBeInTheDocument();
    expect(screen.getByText(/at least three characters/i)).toBeInTheDocument();
  });

  it("answers whose an account is", async () => {
    hooks.search.data = { items: [match()], truncated: false };
    render(<AccountSearchView />);

    await userEvent.type(screen.getByRole("searchbox"), "octocat");

    expect(screen.getByText("octocat")).toBeInTheDocument();
    expect(screen.getByText(/github · gh-main/)).toBeInTheDocument();
    expect(screen.getByText("Ann Lee")).toBeInTheDocument();
  });

  // Nobody holding it is an answer, and a different one from "nobody has
  // decided yet" — leaving the column blank would read as neither.
  it("states an account bound to nobody rather than leaving a gap", () => {
    hooks.search.data = { items: [match({ person: null })], truncated: false };
    render(<AccountSearchView />);

    expect(screen.getByText(/bound to nobody/i)).toBeInTheDocument();
  });

  it("says who decided each binding", () => {
    hooks.search.data = {
      items: [match({ bound_by_operator: true })],
      truncated: false,
    };
    render(<AccountSearchView />);

    expect(screen.getByText(/decided by an operator/i)).toBeInTheDocument();
  });

  it("opens a match in the same case window", async () => {
    hooks.search.data = { items: [match()], truncated: false };
    render(<AccountSearchView />);

    await userEvent.click(screen.getByRole("button", { name: /^open$/i }));

    expect(portalRouter.search.acct).toContain("gh-main");
    expect(
      within(screen.getByRole("dialog")).getByText(/github · gh-main/),
    ).toBeInTheDocument();
  });

  // The search itself proves the account exists — an unbound, never-decided
  // one must open ready to bind, not as "the link may be stale" with no verbs.
  // Binding an unplaced account is this mode's central use case.
  it("opens an unbound, never-decided search hit with the verbs offered", async () => {
    hooks.search.data = { items: [match({ person: null })], truncated: false };
    hooks.binding.data = {
      source: "github",
      source_id: "01900000-0000-7000-8000-00000000aa01",
      account_id: "gh-main",
      person_id: null,
      history: [],
    };
    hooks.binding.isLoading = false;
    render(<AccountSearchView />);

    await userEvent.click(screen.getByRole("button", { name: /^open$/i }));

    const dialog = screen.getByRole("dialog");
    expect(
      within(dialog).queryByText(/link may be stale/i),
    ).not.toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: /detach into a new person/i }),
    ).toBeInTheDocument();
  });

  // An exclusion is an operator's recorded decision; presenting it as
  // "bound to nobody" invites binding the bot and undoing that decision.
  it("shows an excluded account as excluded, not as unbound", () => {
    hooks.search.data = {
      items: [match({ person: null, excluded: true, bound_by_operator: true })],
      truncated: false,
    };
    render(<AccountSearchView />);

    expect(screen.getByText(/excluded — bot \/ CI/i)).toBeInTheDocument();
    expect(screen.queryByText(/bound to nobody/i)).not.toBeInTheDocument();
  });

  it("says a cut list was cut", () => {
    hooks.search.data = { items: [match()], truncated: true };
    render(<AccountSearchView />);

    expect(screen.getByText(/narrow the terms/i)).toBeInTheDocument();
  });
});
