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
import type { AccountMatch, AttentionItem } from "@/api/identity-client";

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
    attention: {
      data: undefined as
        | { items: AttentionItem[]; items_truncated?: boolean; truncated?: boolean }
        | undefined,
      isLoading: false,
      isError: false,
      isPlaceholderData: false,
      refetch: vi.fn(),
    },
    attentionArgs: vi.fn(),
    sources: { data: ["github", "gitlab"] as string[] | undefined, isError: false },
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
  useAttention: (...args: unknown[]) => {
    hooks.attentionArgs(...args);
    return hooks.attention;
  },
  useAccountSources: () => hooks.sources,
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

import { QUEUE_MAX_ITEMS } from "@/api/identity-client";
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

const SOURCE_ID = "01900000-0000-7000-8000-00000000aa01";
const BOB = {
  person_id: "01900000-0000-7000-8000-0000000000b0",
  display_name: "Bob Park",
  email: "bob.park@example.com",
};

function queued(over: Partial<AttentionItem> = {}): AttentionItem {
  return {
    kind: "no_evidence",
    source: "gitlab",
    source_id: SOURCE_ID,
    account_id: "q-1",
    email: null,
    username: "ci-bot-7",
    candidates: [],
    ...over,
  };
}

function unmatched() {
  return screen.getByRole("region", { name: "Unmatched" });
}

function everyAccount() {
  return screen.getByRole("region", { name: "All accounts" });
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
  hooks.attention.data = undefined;
  hooks.attention.isLoading = false;
  hooks.attention.isError = false;
  hooks.attention.isPlaceholderData = false;
  hooks.attentionArgs.mockClear();
  hooks.sources.data = ["github", "gitlab"];
  hooks.sources.isError = false;
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

  it("searches on a single character", async () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    await userEvent.type(screen.getByRole("searchbox"), "o");

    expect(screen.getByText("octocat")).toBeInTheDocument();
  });

  // The needle is one predicate, spaces and all, so a space is a character like
  // any other here — unlike the person search, which matches term by term.
  it("searches a needle that spans a space", async () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    await userEvent.type(screen.getByRole("searchbox"), "a b");

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

describe("AccountSearchView unmatched block", () => {
  it("lists only the three unmatched kinds, in server order", () => {
    hooks.search.data = page([match()]);
    hooks.attention.data = {
      items: [
        queued({ kind: "contested", account_id: "c-1", username: "contested-1" }),
        queued({ kind: "binding_conflict", account_id: "x-1", username: "conflict-1" }),
        queued({ kind: "provisioned_at_login", account_id: "x-2", username: "login-1" }),
        queued({ kind: "minted_from_roster", account_id: "x-3", username: "roster-1" }),
        queued({ kind: "pending", account_id: "x-4", username: "pending-1" }),
        queued({ kind: "no_source_id", account_id: "n-1", username: "nosrc-1" }),
        queued({ kind: "no_evidence", account_id: "e-1", username: "noev-1" }),
      ],
    };
    render(<AccountSearchView />);

    const rows = within(unmatched()).getAllByRole("button");

    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringContaining("contested-1"),
      expect.stringContaining("nosrc-1"),
      expect.stringContaining("noev-1"),
    ]);
    expect(screen.queryByText("pending-1")).not.toBeInTheDocument();
  });

  it("says why each account is unmatched and that it is bound to nobody", () => {
    hooks.attention.data = {
      items: [
        queued({ kind: "contested", account_id: "c-1", username: "contested-1" }),
        queued({ kind: "no_source_id", account_id: "n-1", username: "nosrc-1" }),
        queued({ kind: "no_evidence", account_id: "e-1", username: "noev-1" }),
      ],
    };
    render(<AccountSearchView />);

    const block = within(unmatched());

    expect(block.getByText("More than one person claims it")).toBeInTheDocument();
    expect(block.getByText("No person has this address")).toBeInTheDocument();
    expect(block.getByText("No address to match on")).toBeInTheDocument();
    expect(block.getAllByText(/bound to nobody/i)).toHaveLength(3);
    expect(block.getByText("gitlab · c-1")).toBeInTheDocument();
  });

  it("labels a row by address, then handle, then name, then id", () => {
    hooks.attention.data = {
      items: [
        queued({ account_id: "1", email: "a@example.com", username: "u1", display_name: "N1" }),
        queued({ account_id: "2", email: null, username: "u2", display_name: "N2" }),
        queued({ account_id: "3", email: null, username: null, display_name: "N3" }),
        queued({ account_id: "4", email: null, username: null, display_name: null }),
      ],
    };
    render(<AccountSearchView />);

    const names = within(unmatched())
      .getAllByRole("button")
      .map((r) => r.getAttribute("aria-label")?.split(",")[0]);

    expect(names).toEqual(["a@example.com", "u2", "N3", "4"]);
  });

  it("keeps the heading and says so when the read is complete and nothing is unmatched", () => {
    hooks.search.data = page([match()]);
    hooks.attention.data = { items: [queued({ kind: "pending" })] };
    render(<AccountSearchView />);

    expect(within(unmatched()).getByText("No unmatched accounts")).toBeInTheDocument();
    expect(within(unmatched()).queryByRole("button")).not.toBeInTheDocument();
  });

  it("does not claim nothing is unmatched while the read is not a complete answer", () => {
    for (const [case_, state] of [
      ["loading", { isLoading: true, data: undefined }],
      ["failed", { isError: true, data: undefined }],
      ["capped", { data: { items: [], truncated: true } }],
      ["cut", { data: { items: [], items_truncated: true } }],
    ] as const) {
      Object.assign(hooks.attention, {
        isLoading: false,
        isError: false,
        ...state,
      });
      const { unmount } = render(<AccountSearchView />);

      expect(
        screen.queryByText("No unmatched accounts"),
        `should not say it for: ${case_}`,
      ).not.toBeInTheDocument();
      unmount();
    }
  });

  it("holds the heading with a spinner, and no rows, while the queue is being read", () => {
    hooks.search.data = page([match()]);
    hooks.attention.data = undefined;
    hooks.attention.isLoading = true;
    render(<AccountSearchView />);

    expect(within(unmatched()).getByRole("status")).toBeInTheDocument();
    expect(within(unmatched()).queryByRole("button")).not.toBeInTheDocument();
    expect(screen.queryByText("ci-bot-7")).not.toBeInTheDocument();
  });

  it("keeps the block, with the evidence cap notice, when the read hit its cap and nothing is unmatched", () => {
    hooks.attention.data = { items: [], truncated: true };
    render(<AccountSearchView />);

    expect(
      within(unmatched()).getByText(/the evidence read hit its cap/i),
    ).toBeInTheDocument();
  });

  it("says the evidence read was capped above the rows it lists", () => {
    hooks.attention.data = { items: [queued()], truncated: true };
    render(<AccountSearchView />);

    expect(
      within(unmatched()).getByText(/the evidence read hit its cap/i),
    ).toBeInTheDocument();
    expect(within(unmatched()).getByText("ci-bot-7")).toBeInTheDocument();
  });

  it("narrows to the rows whose shown values contain the typed text, case-insensitively", async () => {
    hooks.attention.data = {
      items: [
        queued({ account_id: "1", email: "ann@example.com", username: "u1" }),
        queued({ account_id: "2", email: null, username: "ci-bot-7" }),
        queued({ account_id: "3", email: null, username: null, display_name: "Zed Quinn" }),
        queued({ account_id: "4", email: null, username: null, source: "hr-sheet" }),
      ],
    };
    render(<AccountSearchView />);
    const field = screen.getByRole("searchbox");

    await userEvent.type(field, "CI-BOT");
    expect(within(unmatched()).getAllByRole("button")).toHaveLength(1);
    expect(within(unmatched()).getByText("ci-bot-7")).toBeInTheDocument();

    await userEvent.clear(field);
    await userEvent.type(field, "quinn");
    expect(within(unmatched()).getByText("Zed Quinn")).toBeInTheDocument();
    expect(within(unmatched()).getAllByRole("button")).toHaveLength(1);

    await userEvent.clear(field);
    await userEvent.type(field, "hr-sheet");
    expect(within(unmatched()).getAllByRole("button")).toHaveLength(1);

    await userEvent.clear(field);
    await userEvent.type(field, "no-such-value");
    expect(within(unmatched()).queryByRole("button")).not.toBeInTheDocument();
    expect(within(unmatched()).getByText("No unmatched accounts")).toBeInTheDocument();

    await userEvent.clear(field);
    expect(within(unmatched()).getAllByRole("button")).toHaveLength(4);
  });

  it("narrows by source the way the listing does, at a segment start only", async () => {
    hooks.attention.data = {
      items: [
        queued({ account_id: "1", username: null, source: "github" }),
        queued({ account_id: "2", username: null, source: "ms_entra" }),
        queued({ account_id: "3", username: null, source: "gitlab-commit-email" }),
      ],
    };
    render(<AccountSearchView />);
    const field = screen.getByRole("searchbox");

    for (const [needle, expected] of [
      ["hub", []],
      ["entra", ["ms_entra · 2"]],
      ["commit", ["gitlab-commit-email · 3"]],
    ] as const) {
      await userEvent.clear(field);
      await userEvent.type(field, needle);

      expect(
        within(unmatched())
          .queryAllByText(/ · /)
          .map((node) => node.textContent),
        `wrong rows for: ${needle}`,
      ).toEqual(expected);
    }
  });

  it("states its own error while the accounts below still list", () => {
    hooks.search.data = page([match()]);
    hooks.attention.isError = true;
    render(<AccountSearchView />);

    expect(within(unmatched()).getByText(/unable to load the review queue/i)).toBeInTheDocument();
    expect(within(everyAccount()).getByText("octocat")).toBeInTheDocument();
  });

  it("retries the read from its error state", async () => {
    hooks.attention.isError = true;
    render(<AccountSearchView />);

    await userEvent.click(within(unmatched()).getByRole("button", { name: /retry|try again/i }));

    expect(hooks.attention.refetch).toHaveBeenCalled();
  });

  it("says the list was cut when the server cut it", () => {
    hooks.attention.data = { items: [queued()], items_truncated: true };
    render(<AccountSearchView />);

    expect(
      within(unmatched()).getByText(/only the first accounts needing review are listed/i),
    ).toBeInTheDocument();
  });

  it("asks for the whole queue narrowed by the picked sources", () => {
    portalRouter.set({ src: "gitlab,github" });
    render(<AccountSearchView />);

    expect(hooks.attentionArgs).toHaveBeenCalledWith(
      QUEUE_MAX_ITEMS,
      ["github", "gitlab"],
      ["contested", "no_source_id", "no_evidence"],
    );
  });

  it("opens an unmatched account in the case window with its candidates", async () => {
    hooks.attention.data = {
      items: [queued({ kind: "contested", account_id: "c-1", username: "contested-1", candidates: [BOB] })],
    };
    hooks.binding.data = {
      source: "gitlab",
      source_id: SOURCE_ID,
      account_id: "c-1",
      person_id: null,
      history: [],
    };
    hooks.binding.isLoading = false;
    render(<AccountSearchView />);

    await userEvent.click(within(unmatched()).getByRole("button", { name: /^contested-1,/ }));

    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Bob Park")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Skip" })).toBeInTheDocument();
  });

  it("prefers the queue row, which carries candidates, when an account is in both blocks", async () => {
    hooks.search.data = page([
      match({ source: "gitlab", source_id: SOURCE_ID, account_id: "c-1", person: null }),
    ]);
    hooks.attention.data = {
      items: [queued({ kind: "contested", account_id: "c-1", username: "contested-1", candidates: [BOB] })],
    };
    hooks.binding.data = {
      source: "gitlab",
      source_id: SOURCE_ID,
      account_id: "c-1",
      person_id: null,
      history: [],
    };
    hooks.binding.isLoading = false;
    render(<AccountSearchView />);

    await userEvent.click(within(everyAccount()).getByRole("button"));

    expect(within(screen.getByRole("dialog")).getByText("Bob Park")).toBeInTheDocument();
  });

  it("offers no Skip on an account opened from All accounts only", async () => {
    hooks.search.data = page([match()]);
    hooks.attention.data = { items: [queued()] };
    hooks.binding.data = {
      source: "github",
      source_id: SOURCE_ID,
      account_id: "gh-main",
      person_id: null,
      history: [],
    };
    hooks.binding.isLoading = false;
    render(<AccountSearchView />);

    await userEvent.click(row());

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Skip" })).not.toBeInTheDocument();
  });
});

describe("AccountSearchView all accounts block", () => {
  it("names the block", () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    expect(within(everyAccount()).getByText("octocat")).toBeInTheDocument();
  });

  it("shows the person an unbound account's address resolves to, as matched by email", () => {
    hooks.search.data = page([
      match({ person: null, address_holder: BOB, bound_by_operator: false }),
    ]);
    hooks.attention.data = { items: [] };
    render(<AccountSearchView />);

    const block = within(everyAccount());

    expect(block.getByText("Bob Park")).toBeInTheDocument();
    expect(block.getByText("matched by email")).toBeInTheDocument();
    expect(block.queryByText(/bound automatically/i)).not.toBeInTheDocument();
    expect(block.queryByText(/bound to nobody/i)).not.toBeInTheDocument();
  });

  it("states bound to nobody, with no holder and no badge, unless the unmatched read is a complete answer", () => {
    for (const [case_, state] of [
      ["loading", { isLoading: true, data: undefined }],
      ["failed", { isError: true, data: undefined }],
      ["not read", { data: undefined }],
      ["truncated", { data: { items: [], truncated: true } }],
      ["items_truncated", { data: { items: [], items_truncated: true } }],
    ] as const) {
      hooks.search.data = page([match({ person: null, address_holder: BOB })]);
      Object.assign(hooks.attention, {
        isLoading: false,
        isError: false,
        ...state,
      });
      const { unmount } = render(<AccountSearchView />);

      const block = within(everyAccount());

      expect(block.queryByText("Bob Park"), `holder shown while: ${case_}`).not.toBeInTheDocument();
      expect(block.queryByText("matched by email"), `badge shown while: ${case_}`).not.toBeInTheDocument();
      expect(block.getByText(/bound to nobody/i), `no nobody while: ${case_}`).toBeInTheDocument();
      unmount();
    }
  });

  it("shows no address holder and no matched-by-email badge on an account that is also unmatched", () => {
    hooks.search.data = page([
      match({
        source: "gitlab",
        source_id: SOURCE_ID,
        account_id: "c-1",
        username: "contested-1",
        person: null,
        address_holder: BOB,
      }),
    ]);
    hooks.attention.data = {
      items: [queued({ kind: "contested", account_id: "c-1", username: "contested-1" })],
    };
    render(<AccountSearchView />);

    const block = within(everyAccount());

    expect(block.queryByText("Bob Park")).not.toBeInTheDocument();
    expect(block.queryByText("matched by email")).not.toBeInTheDocument();
    expect(block.getByText(/bound to nobody/i)).toBeInTheDocument();
  });

  it("says bound automatically for an automatic binding", () => {
    hooks.search.data = page([match()]);
    render(<AccountSearchView />);

    expect(screen.getByText(/bound automatically/i)).toBeInTheDocument();
    expect(screen.queryByText("matched by email")).not.toBeInTheDocument();
  });

  it("says bound to nobody, with no automation or operator badge, when nothing resolves", () => {
    hooks.search.data = page([match({ person: null, address_holder: null })]);
    render(<AccountSearchView />);

    expect(screen.getByText(/bound to nobody/i)).toBeInTheDocument();
    expect(screen.queryByText(/bound automatically/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/decided by an operator/i)).not.toBeInTheDocument();
    expect(screen.queryByText("matched by email")).not.toBeInTheDocument();
  });
});

describe("AccountSearchView sources", () => {
  it("narrows the accounts listing by the picked sources", async () => {
    render(<AccountSearchView />);

    await userEvent.click(screen.getByRole("button", { name: "Sources" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "gitlab" }));

    expect(portalRouter.search.src).toBe("gitlab");
  });

  it("says in the popover that the sources could not be read", async () => {
    hooks.sources.data = undefined;
    hooks.sources.isError = true;
    render(<AccountSearchView />);

    await userEvent.click(screen.getByRole("button", { name: "Sources" }));

    expect(screen.getByText("Sources could not be loaded")).toBeInTheDocument();
    expect(screen.queryByText("No source matches")).not.toBeInTheDocument();
  });

  it("keeps a picked source on the list even when the service no longer reports it", async () => {
    hooks.sources.data = ["github"];
    portalRouter.set({ src: "retired" });
    render(<AccountSearchView />);

    await userEvent.click(screen.getByRole("button", { name: /^Sources/ }));

    expect(screen.getByRole("checkbox", { name: "retired" })).toBeChecked();
  });
});
