// @vitest-environment jsdom
/**
 * The person window. What matters: it mirrors the account window rather than
 * inventing a second idiom — the subject is a person, the list is what they
 * hold, the field searches ACCOUNTS, and a click on a found account BINDS it
 * instead of walking into it; nothing opens a second window over this one; a
 * verb goes through a confirmation and sends the account in the WIRE shape;
 * and a detach that would leave the person with nothing is not offered, with
 * the reason said rather than the button silently missing.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type {
  AccountMatch,
  CorrectionResponse,
  PersonAccountEntry,
} from "@/api/identity-client";

const hooks = vi.hoisted(() => {
  const verb = () => ({
    mutate: vi.fn(),
    reset: vi.fn(),
    isPending: false,
    isError: false,
    error: null as unknown,
  });
  return {
    toast: { success: vi.fn(), error: vi.fn() },
    accounts: {
      data: undefined as
        | { person_id: string; accounts: PersonAccountEntry[] }
        | undefined,
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    },
    search: {
      data: undefined as { pages: { items: AccountMatch[] }[] } | undefined,
      isFetching: false,
      isFetchingNextPage: false,
      isPlaceholderData: false,
      isError: false,
      hasNextPage: false,
      fetchNextPage: vi.fn(),
    },
    bind: verb(),
    detach: verb(),
    exclude: verb(),
  };
});
// The picker debounces its field; identity here keeps the suite about the
// verbs rather than about timers.
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));
vi.mock("@/components/ui/sonner", () => ({ toast: hooks.toast }));
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  usePersonAccounts: () => hooks.accounts,
  useAccountList: () => hooks.search,
  useBindAccount: () => hooks.bind,
  useDetachAccount: () => hooks.detach,
  useExcludeAccount: () => hooks.exclude,
}));

import { PersonDialog } from "./person-dialog";

const ANN = "01900000-0000-7000-8000-0000000000a0";
const BOB = "01900000-0000-7000-8000-0000000000b0";

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

function match(over: Partial<AccountMatch> = {}): AccountMatch {
  return {
    source: "zoom",
    source_id: "01900000-0000-7000-8000-00000000aa03",
    account_id: "zm-9",
    email: null,
    username: "annlee",
    display_name: null,
    person: null,
    bound_by_operator: false,
    ...over,
  };
}

function open(card: { person_id: string; display_name?: string } | null = null) {
  return render(
    <PersonDialog personId={ANN} card={card} onClose={vi.fn()} />,
  );
}

/** The innermost dialog: a confirmation opens over the window, not beside it. */
function confirmation() {
  return screen.getAllByRole("dialog").at(-1) as HTMLElement;
}

beforeEach(() => {
  for (const verb of [hooks.bind, hooks.detach, hooks.exclude]) {
    verb.mutate.mockClear();
    verb.reset.mockClear();
    verb.isPending = false;
    verb.isError = false;
    verb.error = null;
  }
  hooks.toast.success.mockClear();
  hooks.toast.error.mockClear();
  hooks.accounts.data = undefined;
  hooks.accounts.isLoading = false;
  hooks.accounts.isError = false;
  hooks.accounts.refetch.mockClear();
  hooks.search.data = undefined;
  hooks.search.isPlaceholderData = false;
  hooks.search.hasNextPage = false;
});

describe("PersonDialog", () => {
  it("names the person it is about, and their id", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open({ person_id: ANN, display_name: "Ann Lee" });

    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Ann Lee")).toBeInTheDocument();
    expect(within(dialog).getByText(ANN)).toBeInTheDocument();
  });

  // A link resolves an id, not a card — search answers values. The window says
  // what it knows rather than printing the id where a name belongs.
  it("stands the id alone when it arrived without a card", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open(null);

    expect(screen.getByText(/unnamed person/i)).toBeInTheDocument();
  });

  // Moving accounts onto a leaver, or onto a stub automation minted, is the
  // mistake these marks exist to stop — and this window is now where it would
  // be made.
  it("marks a leaver in the heading, the way the roster does", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(
      <PersonDialog
        personId={ANN}
        card={{ person_id: ANN, display_name: "Ann Lee", status: "terminated" }}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByText(/terminated/i)).toBeInTheDocument();
  });

  it("lists every account they hold, saying who decided each", () => {
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
    open();

    expect(screen.getByText("ann@example.com")).toBeInTheDocument();
    expect(screen.getByText(/github · gh-main/)).toBeInTheDocument();
    expect(screen.getByText(/bound automatically/i)).toBeInTheDocument();
    expect(screen.getByText(/decided by an operator/i)).toBeInTheDocument();
  });

  // The governing rule of the redesign: a row here is not a door. Walking into
  // an account from inside a person would stack two windows over one decision.
  it("opens nothing from the accounts it lists", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry(), entry({ account_id: "gh-alt" })] };
    open();

    expect(screen.queryByRole("button", { name: /^open$/i })).not.toBeInTheDocument();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });

  it("states an empty result rather than an empty list", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [] };
    open();

    expect(
      screen.getByText(/no account is bound to this person/i),
    ).toBeInTheDocument();
  });

  it("offers a retry when the read fails", async () => {
    hooks.accounts.isError = true;
    open();

    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(hooks.accounts.refetch).toHaveBeenCalled();
  });

  // A detach mints a person and moves the account there. Taking somebody's ONLY
  // account replaces them with an identical person and leaves their name behind
  // with nothing attached — which is how the accountless persons got made. Said
  // out loud, because a button that is simply absent reads as a fault.
  it("withholds the detach that would leave them with nothing, and says why", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open();

    expect(screen.queryByRole("button", { name: /^detach$/i })).not.toBeInTheDocument();
    expect(screen.getByText(/their only account/i)).toBeInTheDocument();
  });

  it("detaches one of several accounts, in the wire shape, behind a confirmation", async () => {
    hooks.accounts.data = {
      person_id: ANN,
      accounts: [entry(), entry({ account_id: "gh-alt" })],
    };
    open();

    await userEvent.click(screen.getAllByRole("button", { name: /^detach$/i })[0]);
    expect(
      within(confirmation()).getByText(/a new person is created/i),
    ).toBeInTheDocument();
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^detach$/i }),
    );

    expect(hooks.detach.mutate).toHaveBeenCalledWith(
      { account: { source: "github", source_id: entry().source_id, id: "gh-main" } },
      expect.anything(),
    );
  });

  it("excludes an account behind a confirmation", async () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open();

    await userEvent.click(screen.getByRole("button", { name: /^exclude$/i }));
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^exclude$/i }),
    );

    expect(hooks.exclude.mutate).toHaveBeenCalledWith(
      { account: { source: "github", source_id: entry().source_id, id: "gh-main" } },
      expect.anything(),
    );
  });

  // The mirror of the account window's person search: the click picks the other
  // side of a binding. Nothing is written until the confirmation.
  it("binds a found account to this person, and only after a confirmation", async () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    hooks.search.data = { pages: [{ items: [match()] }] };
    open({ person_id: ANN, display_name: "Ann Lee" });

    await userEvent.type(
      screen.getByRole("searchbox", { name: /find an account/i }),
      "annlee",
    );
    await userEvent.click(screen.getByRole("button", { name: /^annlee$/ }));
    expect(hooks.bind.mutate).not.toHaveBeenCalled();

    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^bind$/i }),
    );

    expect(hooks.bind.mutate).toHaveBeenCalledWith(
      {
        account: { source: "zoom", source_id: match().source_id, id: "zm-9" },
        person_id: ANN,
      },
      expect.anything(),
    );
  });

  // Taking an account off somebody else is a different decision from placing an
  // orphan, and the confirmation has to say which one is being taken.
  it("names the person an account would be taken from", async () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    hooks.search.data = {
      pages: [
        { items: [match({ person: { person_id: BOB, display_name: "Bob Park" } })] },
      ],
    };
    open();

    await userEvent.type(
      screen.getByRole("searchbox", { name: /find an account/i }),
      "annlee",
    );
    await userEvent.click(screen.getByRole("button", { name: /^annlee$/ }));

    expect(
      within(confirmation()).getByText(/taken from Bob Park/i),
    ).toBeInTheDocument();
  });

  // The accounts listed above are already theirs: binding one of them again
  // changes nothing, and offering it invites a decision that is not one.
  it("keeps the accounts they already hold out of the search", async () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    hooks.search.data = {
      pages: [{ items: [match(), match({ ...entry(), display_name: null })] }],
    };
    open();

    await userEvent.type(
      screen.getByRole("searchbox", { name: /find an account/i }),
      "ann",
    );

    expect(screen.getByRole("button", { name: /^annlee$/ })).toBeInTheDocument();
    // The held account is listed once — above, as theirs, with its own verbs.
    expect(screen.getAllByText(/github · gh-main/)).toHaveLength(1);
  });

  // The whole fold would bury the handful of accounts the person actually
  // holds, which is what the reader opened them for.
  it("lists no account until the field is asked something", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    hooks.search.data = { pages: [{ items: [match()] }] };
    open();

    expect(screen.queryByText("annlee")).not.toBeInTheDocument();
    expect(screen.getByText("ann@example.com")).toBeInTheDocument();
  });

  // A refusal means the account kept its binding, so the counters stay on
  // screen: the operator still has a decision to take.
  it("keeps the server's counters when an account was refused", async () => {
    const refusal: CorrectionResponse = {
      applied: 0,
      already_decided: 0,
      items: [{ ...entry(), outcome: "refused" }],
    };
    hooks.exclude.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.(refusal),
    );
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open();

    await userEvent.click(screen.getByRole("button", { name: /^exclude$/i }));
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^exclude$/i }),
    );

    expect(screen.getByText(/1 refused/i)).toBeInTheDocument();
    expect(hooks.toast.error).toHaveBeenCalled();
  });
});
