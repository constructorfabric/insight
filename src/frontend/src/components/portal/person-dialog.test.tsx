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

/**
 * The confirmation over the window.
 *
 * An open confirmation hides the window behind it from role queries, so this is
 * normally the only dialog left — but when none opened, `at(-1)` hands back the
 * window itself and the case fails further down for the wrong reason. Its
 * Cancel button is what tells the two apart.
 */
function confirmation() {
  const asked = screen.getAllByRole("dialog").at(-1) as HTMLElement;
  expect(
    within(asked).getByRole("button", { name: "Cancel" }),
  ).toBeInTheDocument();
  return asked;
}

beforeEach(() => {
  for (const verb of [hooks.bind, hooks.detach, hooks.exclude]) {
    // Reset, not clear: `mockClear` keeps an implementation a case installed,
    // and a refusal wired for one verb would then fire in every case after it.
    verb.mutate.mockReset();
    verb.reset.mockReset();
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
  hooks.search.isFetching = false;
  hooks.search.isFetchingNextPage = false;
  hooks.search.isPlaceholderData = false;
  hooks.search.isError = false;
  hooks.search.hasNextPage = false;
  hooks.search.fetchNextPage.mockClear();
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

  // `cn(..., named || fallback)` puts the NAME in the class list when there is
  // one: a display name a connector reported as "hidden" would then hide
  // itself from the heading of the window it is about.
  it("keeps the person's name out of the heading's class list", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(
      <PersonDialog
        personId={ANN}
        card={{ person_id: ANN, display_name: "hidden Lee" }}
        onClose={vi.fn()}
      />,
    );

    const heading = screen.getByText("hidden Lee");
    expect(heading).toBeVisible();
    expect(heading.className).not.toMatch(/hidden/);
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
  // Pressed and keyed, because a row wired as a button answers both.
  it("opens nothing from the accounts it lists", async () => {
    hooks.accounts.data = {
      person_id: ANN,
      accounts: [entry(), entry({ account_id: "gh-alt", email: "alt@example.com" })],
    };
    open();

    const body = screen.getByText("ann@example.com");
    await userEvent.click(body);
    await userEvent.type(body, "{Enter}");

    expect(screen.queryByRole("button", { name: /^open$/i })).not.toBeInTheDocument();
    // No confirmation and no second window: a Cancel button anywhere means
    // something opened. (Counting dialogs would not catch it — an open
    // confirmation hides the window behind it from role queries, so the count
    // stays at one either way.)
    expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /^detach ann@example\.com/i }),
    ).toBeInTheDocument();
    for (const verb of [hooks.bind, hooks.detach, hooks.exclude]) {
      expect(verb.mutate).not.toHaveBeenCalled();
    }
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
  // with nothing attached — which is how the accountless persons got made.
  it("withholds the detach that would leave them with nothing", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open();

    expect(screen.queryByRole("button", { name: /^detach /i })).not.toBeInTheDocument();
    // The row is there, so the verb is withheld rather than the list empty.
    expect(
      screen.getByRole("button", { name: /^exclude ann@example\.com/i }),
    ).toBeInTheDocument();
  });

  it("detaches one of several accounts, in the wire shape, behind a confirmation", async () => {
    hooks.accounts.data = {
      person_id: ANN,
      accounts: [entry(), entry({ account_id: "gh-alt" })],
    };
    open();

    await userEvent.click(screen.getAllByRole("button", { name: /^detach ann@example\.com/i })[0]);
    // This window's own wording, not the account window's: it has to say the
    // person keeps the rest, or the verb reads as "remove from everywhere".
    const asked = within(confirmation());
    expect(asked.getByText(/take this account off this person/i)).toBeInTheDocument();
    expect(asked.getByText(/keeps their other accounts/i)).toBeInTheDocument();
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^detach$/i }),
    );

    expect(hooks.detach.mutate).toHaveBeenCalledWith(
      { account: { source: "github", source_id: entry().source_id, id: "gh-main" } },
      expect.anything(),
    );
  });

  // The modal covers the row that was pressed, and the window's subject is the
  // PERSON — so a confirmation naming only them says nothing about which of
  // several accounts is about to move.
  it.each([/^detach ann@example\.com/i, /^exclude ann@example\.com/i])(
    "names the account the %s confirmation acts on",
    async (verb) => {
      hooks.accounts.data = {
        person_id: ANN,
        accounts: [entry(), entry({ account_id: "gh-alt", email: "alt@example.com" })],
      };
      open();

      await userEvent.click(screen.getAllByRole("button", { name: verb })[0]);

      const asked = within(confirmation());
      expect(asked.getByText("ann@example.com")).toBeInTheDocument();
      expect(asked.getByText(/github · gh-main/)).toBeInTheDocument();
      expect(asked.queryByText("alt@example.com")).not.toBeInTheDocument();
    },
  );

  it("excludes an account behind a confirmation", async () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open();

    await userEvent.click(screen.getByRole("button", { name: /^exclude ann@example\.com/i }));
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
    await userEvent.click(screen.getByRole("button", { name: /^annlee,/ }));
    expect(hooks.bind.mutate).not.toHaveBeenCalled();
    // Both sides of the binding, since the modal hides the list and the field.
    expect(within(confirmation()).getByText(/zoom · zm-9/)).toBeInTheDocument();
    expect(within(confirmation()).getByText("Ann Lee")).toBeInTheDocument();

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
    await userEvent.click(screen.getByRole("button", { name: /^annlee,/ }));

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

    expect(screen.getByRole("button", { name: /^annlee,/ })).toBeInTheDocument();
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

  // Nothing may be fired twice while the first attempt is in flight, and the
  // list behind the confirmation must not offer the same verb again.
  it("locks the verbs while one is in flight", async () => {
    hooks.accounts.data = {
      person_id: ANN,
      accounts: [entry(), entry({ account_id: "gh-alt", email: "alt@example.com" })],
    };
    hooks.exclude.isPending = true;
    open();

    // Every row's verbs, not just the one that fired: the list stays on screen
    // behind the confirmation, and a write in flight is not a moment to start
    // a second one.
    for (const name of [
      /^detach ann@example\.com/i,
      /^exclude ann@example\.com/i,
      /^exclude alt@example\.com/i,
    ]) {
      expect(screen.getByRole("button", { name })).toBeDisabled();
    }
  });

  it("locks the confirmation's own button while its write is in flight", async () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    const { rerender } = open();

    // Opened before the flag is set: the row verb that reaches this dialog is
    // itself disabled while a write is in flight.
    await userEvent.click(
      screen.getByRole("button", { name: /^exclude ann@example\.com/i }),
    );
    hooks.exclude.isPending = true;
    rerender(<PersonDialog personId={ANN} card={null} onClose={vi.fn()} />);

    expect(
      within(confirmation()).getByRole("button", { name: /^exclude$/i }),
    ).toBeDisabled();
  });

  // Closing over an unshown error would read as success.
  it("states a failed verb inside the confirmation it was fired from", async () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    hooks.exclude.isError = true;
    hooks.exclude.error = new Error("boom");
    open();

    await userEvent.click(
      screen.getByRole("button", { name: /^exclude ann@example\.com/i }),
    );

    expect(
      within(confirmation()).getByText(/was not applied/i),
    ).toBeInTheDocument();
  });

  // A dialog's error belongs to the attempt made in THAT dialog: without the
  // reset the next one opens already wearing the previous failure.
  it("resets every verb when a confirmation is dismissed", async () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open();

    await userEvent.click(
      screen.getByRole("button", { name: /^exclude ann@example\.com/i }),
    );
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: "Cancel" }),
    );

    for (const verb of [hooks.bind, hooks.detach, hooks.exclude]) {
      expect(verb.reset).toHaveBeenCalled();
    }
  });

  // The window stays open — the list under it re-reads and that IS the answer —
  // but the confirmation goes, and nothing claims a refusal that did not happen.
  it("closes the confirmation and shows no counters when everything applied", async () => {
    hooks.exclude.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.({
          applied: 1,
          already_decided: 0,
          items: [{ ...entry(), outcome: "applied" }],
        }),
    );
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open();

    await userEvent.click(
      screen.getByRole("button", { name: /^exclude ann@example\.com/i }),
    );
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^exclude$/i }),
    );

    expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
    expect(screen.queryByText(/refused/i)).not.toBeInTheDocument();
    expect(hooks.toast.success).toHaveBeenCalled();
    expect(hooks.toast.error).not.toHaveBeenCalled();
  });

  // A detach mints a person, and this window stays open rather than handing the
  // id to a window that closes — the toast is the only place it can be read, so
  // it carries the id and stays up long enough to copy it.
  it("reports the minted person id, and keeps that toast up", async () => {
    const minted = "01900000-0000-7000-8000-0000000000c0";
    hooks.detach.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.({
          applied: 1,
          already_decided: 0,
          items: [{ ...entry(), outcome: "applied" }],
          new_person_id: minted,
        }),
    );
    hooks.accounts.data = {
      person_id: ANN,
      accounts: [entry(), entry({ account_id: "gh-alt", email: "alt@example.com" })],
    };
    open();

    await userEvent.click(
      screen.getByRole("button", { name: /^detach ann@example\.com/i }),
    );
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^detach$/i }),
    );

    expect(hooks.toast.success).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        description: expect.stringContaining(minted),
        duration: expect.any(Number),
      }),
    );
  });

  // Nothing was decided, so the counters are the answer and the window keeps
  // its verbs.
  it("reports an account that had already moved without claiming a refusal", async () => {
    hooks.exclude.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.({
          applied: 0,
          already_decided: 1,
          items: [{ ...entry(), outcome: "already_decided" }],
        }),
    );
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    open();

    await userEvent.click(
      screen.getByRole("button", { name: /^exclude ann@example\.com/i }),
    );
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^exclude$/i }),
    );

    expect(hooks.toast.success).toHaveBeenCalled();
    expect(screen.queryByText(/refused/i)).not.toBeInTheDocument();
  });

  it("spins while the accounts are still being read", () => {
    hooks.accounts.isLoading = true;
    open();

    expect(within(screen.getByRole("dialog")).getByRole("status")).toBeInTheDocument();
  });

  // A card for a DIFFERENT person is not this person's card: captioning the
  // window with it would name the person the reader looked at before.
  it("ignores a card that belongs to somebody else", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(
      <PersonDialog
        personId={ANN}
        card={{ person_id: BOB, display_name: "Bob Park" }}
        onClose={vi.fn()}
      />,
    );

    expect(screen.queryByText("Bob Park")).not.toBeInTheDocument();
    expect(screen.getByText(/unnamed person/i)).toBeInTheDocument();
  });

  // Binding accounts onto a stub automation minted is the wrong direction —
  // the history is on the other side.
  it("marks a provisional person in the heading", () => {
    hooks.accounts.data = { person_id: ANN, accounts: [entry()] };
    render(
      <PersonDialog
        personId={ANN}
        card={{ person_id: ANN, display_name: "Ann Lee", provisional: true }}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByText(/provisional/i)).toBeInTheDocument();
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

    await userEvent.click(screen.getByRole("button", { name: /^exclude ann@example\.com/i }));
    await userEvent.click(
      within(confirmation()).getByRole("button", { name: /^exclude$/i }),
    );

    expect(screen.getByText(/1 refused/i)).toBeInTheDocument();
    expect(hooks.toast.error).toHaveBeenCalled();
  });
});
