// @vitest-environment jsdom
/**
 * The account mode. What matters: an operator holding a handle or an address
 * learns whose it is — the question neither other mode can answer, since both
 * are entered through a person; unbound is stated as an answer rather than
 * left blank; the ROW itself opens the account in the same case window, the way
 * the queue's rows do, so one gesture means one thing across the console; and
 * with nothing typed the mode lists what the connectors reported instead of
 * waiting to be asked.
 */
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
      isPlaceholderData: false,
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
// The debounce is a pure timing concern with its own unit test; identity here
// keeps this suite about behaviour, not timers.
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));
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

import { MAX_SEARCH_CHARS } from "@/queries/identity-resolution";
import { portalRouter } from "@/test/portal-router";

import { scrollEndIntoView } from "@/test/intersection-observer";

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

/** The row for {@link match}, addressed the way a reader reaches it. */
function row() {
  return screen.getByRole("button", { name: /^octocat,/ });
}

afterEach(() => window.getSelection()?.removeAllRanges());

beforeEach(() => {
  hooks.search.data = undefined;
  hooks.search.isFetching = false;
  hooks.search.isFetchingNextPage = false;
  hooks.search.isPlaceholderData = false;
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

  // One letter reaches most of what the connectors reported and costs the fold a
  // pass to say so. Going silent would read as a broken field, so the mode says
  // what it is waiting for — and shows no rows it did not ask for.
  // The service refuses a needle past 200 characters, so a field that accepted
  // one would turn an ordinary paste — an id, a url, a line from a log — into a
  // refusal the operator has no way to read as "too long".
  it("stops at the length the service accepts", () => {
    render(<AccountSearchView />);

    expect(screen.getByRole("searchbox")).toHaveAttribute(
      "maxlength",
      String(MAX_SEARCH_CHARS),
    );
  });

  it("asks for a second character instead of searching on one", async () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    await userEvent.type(screen.getByRole("searchbox"), "o");

    expect(screen.getByText(/at least 2 characters/i)).toBeInTheDocument();
    expect(screen.queryByText("octocat")).not.toBeInTheDocument();
    // Neither emptiness applies while the field is still being typed into.
    expect(screen.queryByText(/nothing to list here yet/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/carries that/i)).not.toBeInTheDocument();
  });

  it("searches, and drops the notice, on the second character", async () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    await userEvent.type(screen.getByRole("searchbox"), "oc");

    expect(screen.queryByText(/at least 2 characters/i)).not.toBeInTheDocument();
    expect(screen.getByText("octocat")).toBeInTheDocument();
  });

  // The needle is one predicate, spaces and all, so a space is a character like
  // any other here — unlike the person search, which matches term by term.
  it("searches a needle that spans a space", async () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    await userEvent.type(screen.getByRole("searchbox"), "a b");

    expect(screen.queryByText(/at least 2 characters/i)).not.toBeInTheDocument();
    expect(screen.getByText("octocat")).toBeInTheDocument();
  });

  // Kept rows are the previous needle's answer for one debounce, so an empty
  // list is not yet a fact about the field: "nothing to list here yet" would
  // read as "no connector has reported", which is a claim about the tenant.
  it("claims neither emptiness while the listing is still the previous answer", () => {
    hooks.search.data = page([]);
    hooks.search.isPlaceholderData = true;
    render(<AccountSearchView />);

    expect(screen.queryByText(/nothing to list here yet/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/carries that/i)).not.toBeInTheDocument();
  });

  // Candidates answer "whose COULD it be", which is a queue question. This list
  // answers "whose IS it", so the window it opens must carry no candidates —
  // otherwise the holder appears as somebody to bind the account to, which is
  // the confirm act and belongs in the queue.
  it("opens a listed account with no candidates to confirm", async () => {
    hooks.search.data = page([match()]);
    // A real binding, or the window body is a spinner and this proves nothing.
    hooks.binding.data = {
      source: "github",
      source_id: "01900000-0000-7000-8000-00000000aa01",
      account_id: "gh-main",
      person_id: "01900000-0000-7000-8000-0000000000a0",
      history: [],
    };
    hooks.binding.isLoading = false;
    render(<AccountSearchView />);

    await userEvent.click(row());

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

    await userEvent.click(row());

    expect(portalRouter.search.acct).toContain("gh-main");
    expect(
      within(screen.getByRole("dialog")).getByText(/github · gh-main/),
    ).toBeInTheDocument();
  });

  // The row IS the door, so a button beside it would be a second way through
  // the same one — and the queue's rows have never had one.
  it("carries no open button beside the row that already opens", () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    // The positive control: without it this case also passes for a row that
    // failed to render at all.
    expect(row()).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^open$/i }),
    ).not.toBeInTheDocument();
  });

  // The addresses and ids on these rows are what an operator copies into a
  // ticket. A row that opens on the mouse-up of a drag makes them unreadable.
  it("does not open on the click that ends a text selection", () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);
    const selected = document.createRange();
    selected.selectNodeContents(screen.getByText("octocat"));
    const selection = window.getSelection();
    // A Selection holds one range: adding to a stale one from an earlier case
    // is a no-op, and this case would then be asserting nothing.
    selection?.removeAllRanges();
    selection?.addRange(selected);

    // The bare click, not a full press: a real drag ends in a mouseup with the
    // selection still standing, and a synthesized mousedown would clear it
    // before the handler ever sees it.
    fireEvent.click(row());

    expect(portalRouter.search.acct).toBeUndefined();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // The same click with nothing selected DOES open — without this the case
    // would still pass for a row that never opens at all.
    selection?.removeAllRanges();
    fireEvent.click(row());

    expect(portalRouter.search.acct).toContain("gh-main");
  });

  // The holder's card carries a copy control, and pressing it is not a request
  // to leave the list.
  it("does not open when a control inside the row is pressed", async () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    await userEvent.click(
      within(row()).getByRole("button", { name: /copy/i }),
    );

    expect(portalRouter.search.acct).toBeUndefined();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // The row itself still opens, so the guard above is a guard and not a row
    // that never worked.
    await userEvent.click(row());

    expect(portalRouter.search.acct).toContain("gh-main");
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

    await userEvent.click(row());

    const dialog = screen.getByRole("dialog");
    expect(
      within(dialog).queryByText(/link may be stale/i),
    ).not.toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: /exclude \(bot/i }),
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
  // Reading the fold means scrolling it, so the next page comes on the way down
  // rather than behind a button at the bottom of the one before it.
  it("asks for the next page when the end of the list comes into view", () => {
    hooks.search.data = page([match()]);
    hooks.search.hasNextPage = true;
    render(<AccountSearchView />);

    scrollEndIntoView();

    expect(hooks.search.fetchNextPage).toHaveBeenCalled();
  });

  it("stops asking once the listing says there is no next page", () => {
    hooks.search.data = page([match()]);
    hooks.search.hasNextPage = false;
    render(<AccountSearchView />);

    scrollEndIntoView();

    expect(hooks.search.fetchNextPage).not.toHaveBeenCalled();
  });
});
