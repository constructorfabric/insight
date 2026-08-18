// @vitest-environment jsdom
/**
 * The review queue. What matters: the empty queue is a celebrated goal state,
 * not a blank table; groups come in working order with honest counts and an
 * unknown kind still shows up (the vocabulary is open by contract); accounts
 * arguing over the same people are ONE case rather than as many rows as the
 * server sends; selection lives in the URL so an operator can share a link;
 * and the strip leads with the queue's own size — the one figure the operator
 * can act on — over tenant-wide binding states.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AttentionItem, AttentionResponse } from "@/api/identity-client";

vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

const attention = vi.hoisted(() => ({
  q: {
    data: undefined as AttentionResponse | undefined,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  },
}));
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  useAttention: () => attention.q,
  useAccountList: () => ({
    data: undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  }),
  // The people mode has its own test file; here it only has to mount.
  usePersonList: () => ({
    data: undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  }),
  usePersonAccounts: () => ({
    data: undefined,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  }),
  // The panel under a selection; its own behaviour is account-detail.test's.
  useAccountBinding: () => ({
    data: undefined,
    isLoading: true,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
}));

import { portalRouter } from "@/test/portal-router";

import { IdentitiesView } from "./identities-view";

const RATES = { observed: 60, bound: 55, pending: 3, no_evidence: 1, excluded: 1 };

function item(over: Partial<AttentionItem>): AttentionItem {
  return {
    kind: "contested",
    source: "github",
    source_id: "01900000-0000-7000-8000-00000000aa01",
    account_id: "dev-42",
    email: "dev42@example.com",
    username: null,
    candidates: [],
    ...over,
  };
}

beforeEach(() => {
  attention.q.data = undefined;
  attention.q.isLoading = false;
  attention.q.isError = false;
  attention.q.refetch.mockClear();
  portalRouter.reset();
  portalRouter.set({ zone: "manage", item: "identities" });
});

describe("IdentitiesView", () => {
  it("celebrates the empty queue instead of rendering a blank table", () => {
    attention.q.data = { items: [], rates: RATES };
    render(<IdentitiesView />);

    expect(screen.getByText(/everything is resolved/i)).toBeInTheDocument();
    // The rates strip still shows the tenant-wide picture.
    expect(screen.getByText("60")).toBeInTheDocument();
  });

  // An emptied backlog is exactly when a colleague opens the link they were
  // sent, so the celebration must not take the detail panel down with it.
  // `role="status"` is the panel's own loading state (the binding query is
  // mocked pending here) — its presence means AccountDetail mounted.
  it("answers a shared ?acct= link even after the backlog is worked to zero", () => {
    attention.q.data = { items: [], rates: RATES };
    portalRouter.set({
      zone: "manage",
      item: "identities",
      acct: "github:01900000-0000-7000-8000-00000000aa01:dev-42",
    });
    render(<IdentitiesView />);

    expect(screen.getByText(/everything is resolved/i)).toBeInTheDocument();
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("keeps a bare empty queue a plain celebration — no panel, no placeholder", () => {
    attention.q.data = { items: [], rates: RATES };
    render(<IdentitiesView />);

    expect(screen.getByText(/everything is resolved/i)).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(screen.queryByText(/pick an account from the queue/i)).not.toBeInTheDocument();
  });

  it("says so when the server truncated the evidence — partial counts must not read as tenant-wide", () => {
    attention.q.data = { items: [], rates: RATES, truncated: true };
    render(<IdentitiesView />);

    expect(screen.getByText(/cover only part of the observed accounts/i)).toBeInTheDocument();
  });

  // Two different facts: the evidence read hit its ceiling (rates are a
  // prefix) versus the item cap cut the list (rates still whole-tenant).
  it("says the list was cut when only the item cap was hit", () => {
    attention.q.data = { items: [], rates: RATES, items_truncated: true };
    render(<IdentitiesView />);

    expect(screen.getByText(/only the first accounts needing review/i)).toBeInTheDocument();
    expect(
      screen.queryByText(/cover only part of the observed accounts/i),
    ).not.toBeInTheDocument();
  });

  it("does not repeat the item-cap notice when the evidence read was truncated too", () => {
    attention.q.data = {
      items: [],
      rates: RATES,
      truncated: true,
      items_truncated: true,
    };
    render(<IdentitiesView />);

    expect(screen.getByText(/cover only part of the observed accounts/i)).toBeInTheDocument();
    expect(
      screen.queryByText(/only the first accounts needing review/i),
    ).not.toBeInTheDocument();
  });

  it("shows no truncation warning on a complete read, including from an older backend", () => {
    attention.q.data = { items: [], rates: RATES };
    render(<IdentitiesView />);

    expect(
      screen.queryByText(/cover only part of the observed accounts/i),
    ).not.toBeInTheDocument();
  });

  it("groups by kind with honest counts, and an unknown kind still shows", () => {
    attention.q.data = {
      items: [
        item({ account_id: "a1" }),
        item({ account_id: "a2" }),
        item({ kind: "no_evidence", account_id: "bot-1", email: null, username: "bot-1" }),
        item({ kind: "quarantined", account_id: "q-1", email: null, username: null }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    const contested = screen.getByText(/contested/i).closest("[data-slot=card]");
    expect(within(contested as HTMLElement).getByText("2")).toBeInTheDocument();
    expect(screen.getByText(/no address to match on/i)).toBeInTheDocument();
    // Unknown kind lands in the catch-all group rather than vanishing.
    expect(screen.getByText("q-1")).toBeInTheDocument();
  });

  // Five rows repeating the same two candidates read as five problems. The
  // people are stated once for the case; the rows underneath are the accounts
  // each decision is taken on.
  it("shows one case for the accounts arguing over the same people", () => {
    const candidates = [
      { person_id: "01900000-0000-7000-8000-0000000000a0", display_name: "Ann Lee" },
      { person_id: "01900000-0000-7000-8000-0000000000b0", display_name: "Bob Park" },
    ];
    attention.q.data = {
      items: [
        item({
          kind: "binding_conflict",
          account_id: "a1",
          source: "hr",
          candidates,
          bound_to: candidates[0]?.person_id,
        }),
        item({
          kind: "binding_conflict",
          account_id: "a2",
          source: "wiki",
          candidates,
          bound_to: candidates[1]?.person_id,
        }),
        item({
          kind: "binding_conflict",
          account_id: "a3",
          source: "chat",
          candidates,
          bound_to: candidates[1]?.person_id,
        }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    expect(screen.getAllByText("Ann Lee")).toHaveLength(1);
    expect(screen.getByText(/1 case · 3 accounts/i)).toBeInTheDocument();
    // The candidates are stated once for the case, so each row has to say
    // which of them it would be taking the account from.
    expect(screen.getByText(/held by Ann Lee/i)).toBeInTheDocument();
    expect(screen.getAllByText(/held by Bob Park/i)).toHaveLength(2);
    // Each account still has its own row: a decision is taken per account.
    expect(screen.getAllByRole("button", { name: /dev42@example\.com/i })).toHaveLength(3);
  });

  // Nothing here is matchable, which is exactly why it is on the queue: only
  // a person can bind these, and an account id names nobody.
  it("names an account with nothing to match on by what the source says it is", () => {
    attention.q.data = {
      items: [
        item({
          kind: "no_evidence",
          account_id: "921",
          email: null,
          username: null,
          display_name: "Ann Lee",
          job_title: "Engineer",
          department: "Platform",
          status: "Inactive",
          manager_email: "lead@example.com",
        }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    const row = screen.getByRole("button", { name: /ann lee/i });
    expect(within(row).getByText(/Engineer · Platform/)).toBeInTheDocument();
    expect(within(row).getByText(/reports to lead@example\.com/)).toBeInTheDocument();
    // A leaver is rarely the operator's work; the queue must not hide it.
    expect(within(row).getByText("Inactive")).toBeInTheDocument();
    // The id still identifies the account — beside its source, not as a name.
    expect(within(row).getByText(/github · 921/)).toBeInTheDocument();
  });

  // First-login provisioning trades "cannot sign in" for "possibly a duplicate
  // person". Bound is not decided: without a group of its own the trade would
  // leave the queue silently, at the moment the account gained a live owner.
  it("keeps a login-minted binding on the queue until a human decides it", () => {
    attention.q.data = {
      items: [
        item({
          kind: "provisioned_at_login",
          account_id: "new-joiner",
          email: null,
          username: "new-joiner",
          bound_to: "01900000-0000-7000-8000-0000000000c0",
          candidates: [
            {
              person_id: "01900000-0000-7000-8000-0000000000c0",
              display_name: "Carol Chen",
            },
          ],
        }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    expect(screen.getByText(/given a person at first sign-in/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /new-joiner/i })).toBeInTheDocument();
  });

  it("renders candidates as person cells", () => {
    attention.q.data = {
      items: [
        item({
          candidates: [
            {
              person_id: "01900000-0000-7000-8000-000000000001",
              display_name: "Bob Park",
              email: "bob.park@example.com",
            },
          ],
        }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    expect(screen.getByText("Bob Park")).toBeInTheDocument();
  });

  it("writes the selection into the URL and toggles it off on a second click", async () => {
    attention.q.data = { items: [item({})], rates: RATES };
    render(<IdentitiesView />);

    const row = screen.getByRole("button", { name: /dev42@example\.com/i });
    await userEvent.click(row);
    expect(portalRouter.search.acct).toContain("dev-42");
    expect(row).toHaveAttribute("aria-pressed", "true");

    await userEvent.click(row);
    expect(portalRouter.search.acct).toBeUndefined();
  });

  // The tiles count binding states; only the queue is work. A tile promising
  // "review" for accounts the resolver binds by itself sent an operator
  // looking for something they cannot do.
  it("leads with the number the operator can act on — the queue's own size", () => {
    attention.q.data = { items: [item({}), item({ account_id: "a2" })], rates: RATES };
    render(<IdentitiesView />);

    const tile = screen
      .getByText(/needs a decision/i)
      .closest("div")?.parentElement;
    expect(within(tile as HTMLElement).getByText("2")).toBeInTheDocument();
    // The state tiles say what they count, never "review".
    expect(screen.getByText(/unbound · has an address/i)).toBeInTheDocument();
    expect(screen.queryByText(/pending review/i)).not.toBeInTheDocument();
  });

  it("marks the decision count as a floor when the server cut the list", () => {
    attention.q.data = { items: [item({})], rates: RATES, items_truncated: true };
    render(<IdentitiesView />);

    expect(screen.getByText("1+")).toBeInTheDocument();
  });

  // The row carries the values an operator copies out, so it cannot be a
  // <button>: its text would not be selectable and the cards' copy controls
  // would be interactive content nested inside a control.
  it("keeps a copy press inside a row from selecting the case", async () => {
    attention.q.data = {
      items: [
        item({
          candidates: [
            { person_id: "01900000-0000-7000-8000-000000000001", display_name: "Bob Park" },
          ],
        }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    await userEvent.click(
      screen.getByRole("button", { name: /copy 01900000-0000-7000-8000-000000000001/i }),
    );
    expect(portalRouter.search.acct).toBeUndefined();
  });

  // The case is where every decision is taken, so it gets a window rather
  // than the leftover column — and the window is opened by the URL, so a
  // shared link lands a colleague on the same case rather than on a queue.
  it("opens the case in a window, and closing it clears the shared link", async () => {
    attention.q.data = { items: [item({})], rates: RATES };
    render(<IdentitiesView />);

    await userEvent.click(screen.getByRole("button", { name: /dev42@example\.com/i }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText(/github · dev-42/i)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /close/i }));
    expect(portalRouter.search.acct).toBeUndefined();
  });

  it("narrows the queue by anything on a row, and carries the filter in the URL", async () => {
    attention.q.data = {
      items: [
        item({ account_id: "a1", email: "ann@example.com" }),
        item({ account_id: "a2", email: "bob@example.com" }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    await userEvent.type(screen.getByRole("searchbox"), "ann@");
    await waitFor(() => expect(portalRouter.search.filter).toBe("ann@"));
    expect(screen.getByRole("button", { name: /ann@example\.com/i })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /bob@example\.com/i }),
    ).not.toBeInTheDocument();
  });

  // Celebrating here would tell an operator the backlog is done because they
  // mistyped a filter.
  it("does not celebrate an empty result — a filter that matches nothing says so", () => {
    attention.q.data = { items: [item({})], rates: RATES };
    portalRouter.set({ zone: "manage", item: "identities", filter: "nobody" });
    render(<IdentitiesView />);

    expect(screen.getByText(/nothing matches those terms/i)).toBeInTheDocument();
    expect(screen.queryByText(/everything is resolved/i)).not.toBeInTheDocument();
  });

  // A colleague's link points at a row the reader's own filter hides; the
  // case must still open, or the link is only as good as the recipient's
  // current view.
  it("answers a shared ?acct= link even while a filter hides its row", () => {
    attention.q.data = { items: [item({})], rates: RATES };
    portalRouter.set({
      zone: "manage",
      item: "identities",
      filter: "nobody",
      acct: "github:01900000-0000-7000-8000-00000000aa01:dev-42",
    });
    render(<IdentitiesView />);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  // A backlog is worked in one pass: the list moves like a list, and closing
  // a case puts the operator back where they were rather than at the top.
  it("moves between rows with the arrow keys", async () => {
    attention.q.data = {
      items: [
        item({ account_id: "a1", email: "ann@example.com" }),
        item({ account_id: "a2", email: "bob@example.com" }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    const first = screen.getByRole("button", { name: /ann@example\.com/i });
    first.focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(screen.getByRole("button", { name: /bob@example\.com/i })).toHaveFocus();

    await userEvent.keyboard("{ArrowUp}");
    expect(first).toHaveFocus();
  });

  it("returns focus to the row when the case window closes", async () => {
    attention.q.data = { items: [item({})], rates: RATES };
    render(<IdentitiesView />);

    const row = screen.getByRole("button", { name: /dev42@example\.com/i });
    await userEvent.click(row);
    await userEvent.click(screen.getByRole("button", { name: /close/i }));

    expect(row).toHaveFocus();
  });

  it("offers the next account without a trip back to the list", async () => {
    attention.q.data = {
      items: [
        item({ account_id: "a1", email: "ann@example.com" }),
        item({ account_id: "a2", email: "bob@example.com" }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    await userEvent.click(screen.getByRole("button", { name: /ann@example\.com/i }));
    await userEvent.click(screen.getByRole("button", { name: /next account/i }));

    expect(portalRouter.search.acct).toContain("a2");
  });

  // A decision prunes its row from the list at once. The window must hold
  // what it knew — the case and its position — or the operator's own success
  // kills the outcome they are reading and the Next button they are about to
  // press.
  it("keeps the open case and the conveyor when the decided row is pruned", async () => {
    attention.q.data = {
      items: [
        item({ account_id: "a1", email: "ann@example.com" }),
        item({ account_id: "a2", email: "bob@example.com" }),
      ],
      rates: RATES,
    };
    const view = render(<IdentitiesView />);

    await userEvent.click(screen.getByRole("button", { name: /ann@example\.com/i }));
    // The verb landed: the server said "decided", the cache dropped the row.
    attention.q.data = {
      items: [item({ account_id: "a2", email: "bob@example.com" })],
      rates: RATES,
    };
    view.rerender(<IdentitiesView />);

    const dialog = screen.getByRole("dialog");
    // The case still names what was decided, not a stale-link apology.
    expect(within(dialog).getByText(/ann@example\.com/)).toBeInTheDocument();
    expect(within(dialog).queryByText(/link may be stale/i)).not.toBeInTheDocument();
    // And the conveyor still moves: the row that shifted into this slot is next.
    await userEvent.click(within(dialog).getByRole("button", { name: /next account/i }));
    expect(portalRouter.search.acct).toContain("a2");
  });

  // Modes are ways IN to the same decisions; the mode rides in the URL so a
  // link opens the one it was sent from.
  it("switches modes through the URL, dropping the account selected in the old one", async () => {
    attention.q.data = { items: [item({})], rates: RATES };
    // A selection the window cannot open (a mistyped link) leaves the value in
    // the URL with no modal over the tabs — the state the switch must clear.
    portalRouter.set({ zone: "manage", item: "identities", acct: "malformed" });
    render(<IdentitiesView />);

    await userEvent.click(screen.getByRole("tab", { name: /a person and their accounts/i }));

    expect(portalRouter.search.mode).toBe("people");
    // A case picked in the queue means nothing in a list it is not part of.
    expect(portalRouter.search.acct).toBeUndefined();
    // The mode it switched TO is on screen, so the cleared selection is not
    // just an empty URL over the old surface.
    expect(
      screen.getByRole("searchbox", { name: /search people/i }),
    ).toBeInTheDocument();
  });

  it("falls back to the queue when the URL names a mode that does not exist", () => {
    attention.q.data = { items: [item({})], rates: RATES };
    portalRouter.set({ zone: "manage", item: "identities", mode: "not-a-mode" });
    render(<IdentitiesView />);

    expect(screen.getByRole("button", { name: /dev42@example\.com/i })).toBeInTheDocument();
  });

  it("offers a retry on a failed load", async () => {
    attention.q.isError = true;
    render(<IdentitiesView />);

    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(attention.q.refetch).toHaveBeenCalled();
  });
});
