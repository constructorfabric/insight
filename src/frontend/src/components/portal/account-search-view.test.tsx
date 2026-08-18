// @vitest-environment jsdom
/**
 * The account mode. What matters: an operator holding a handle or an address
 * learns whose it is — the question neither other mode can answer, since both
 * are entered through a person; unbound is stated as an answer rather than
 * left blank; the account opens in the same case window, so the verbs are one
 * click away; and with nothing typed the mode lists what the connectors
 * reported instead of waiting to be asked.
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
      data: undefined as
        | { pages: { items: AccountMatch[] }[] }
        | undefined,
      isFetching: false,
      isFetchingNextPage: false,
      isError: false,
      hasNextPage: false,
      fetchNextPage: vi.fn(),
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
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  useAccountList: () => hooks.search,
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
  usePersonList: () => ({
    data: undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  }),
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

function page(items: AccountMatch[]) {
  return { pages: [{ items }] };
}

beforeEach(() => {
  hooks.search.data = undefined;
  hooks.search.isFetching = false;
  hooks.search.isFetchingNextPage = false;
  hooks.search.isError = false;
  hooks.search.hasNextPage = false;
  hooks.search.fetchNextPage = vi.fn();
  hooks.binding.data = undefined;
  hooks.binding.isLoading = true;
  portalRouter.reset();
  portalRouter.set({ zone: "manage", item: "identities", mode: "accounts" });
});

describe("AccountSearchView", () => {
  // Waiting to be asked hides exactly the accounts nobody thinks to search
  // for; the mode opens on what the connectors reported.
  it("lists the observed accounts before anything is typed", () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    expect(screen.getByText("octocat")).toBeInTheDocument();
  });

  // An empty list is not proof of an empty tenant: the service answers one for
  // a fold it cannot read yet, so this state says what it knows — nothing to
  // list — and names the other explanation instead of denying it.
  it("offers nothing to list without claiming no account exists", () => {
    hooks.search.data = page([]);
    render(<AccountSearchView />);

    expect(screen.getByText(/nothing to list here yet/i)).toBeInTheDocument();
    expect(screen.getByText(/may not have run yet/i)).toBeInTheDocument();
    expect(
      screen.queryByText(/no account has been observed/i),
    ).not.toBeInTheDocument();
  });

  it("answers whose an account is", async () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    await userEvent.type(screen.getByRole("searchbox"), "octocat");

    expect(screen.getByText("octocat")).toBeInTheDocument();
    expect(screen.getByText(/github · gh-main/)).toBeInTheDocument();
    expect(screen.getByText("Ann Lee")).toBeInTheDocument();
  });

  // Nobody holding it is an answer, and a different one from "nobody has
  // decided yet" — leaving the column blank would read as neither.
  it("states an account bound to nobody rather than leaving a gap", () => {
    hooks.search.data = page([match({ person: null })]);
    render(<AccountSearchView />);

    expect(screen.getByText(/bound to nobody/i)).toBeInTheDocument();
  });

  it("says who decided each binding", () => {
    hooks.search.data = page([match({ bound_by_operator: true })]);
    render(<AccountSearchView />);

    expect(screen.getByText(/decided by an operator/i)).toBeInTheDocument();
  });

  it("opens a match in the same case window", async () => {
    hooks.search.data = page([match()]);
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
    hooks.search.data = page([match({ person: null })]);
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
    hooks.search.data = page([
      match({ person: null, excluded: true, bound_by_operator: true }),
    ]);
    render(<AccountSearchView />);

    expect(screen.getByText(/excluded — bot \/ CI/i)).toBeInTheDocument();
    expect(screen.queryByText(/bound to nobody/i)).not.toBeInTheDocument();
  });

  // A cut list used to end the road; now it continues, so the control is an
  // offer to read on rather than an instruction to retype.
  it("offers the next page instead of asking for narrower terms", async () => {
    hooks.search.data = page([match()]);
    hooks.search.hasNextPage = true;
    render(<AccountSearchView />);

    await userEvent.click(screen.getByRole("button", { name: /show more/i }));

    expect(hooks.search.fetchNextPage).toHaveBeenCalled();
  });
});
