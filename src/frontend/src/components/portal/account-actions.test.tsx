// @vitest-environment jsdom
/**
 * The decision surface. What matters: the button on the currently bound
 * candidate reads "Confirm" (re-asserting IS the decision) while others read
 * "Bind"; merge is NOT offered here at all (it is a claim about people — see
 * `merge-case-dialog.test.tsx`); every verb goes through a confirmation and
 * sends the account in the WIRE shape (`id`, not
 * `account_id`); a verb that decided everything it named hands the window back
 * instead of leaving a list the server has moved past on screen; and a refusal
 * keeps the window, with the server's counters verbatim.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AccountBinding, CorrectionResponse } from "@/api/identity-client";

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
    bind: verb(),
    detach: verb(),
    /** What else the holder has — decides whether a detach is offered. */
    owned: {
      data: undefined as { person_id: string; accounts: unknown[] } | undefined,
    },
    exclude: verb(),
    search: {
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
// The picker inside this component debounces its field; identity here keeps the
// suite about the verbs rather than about timers.
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));
vi.mock("sonner", () => ({ toast: hooks.toast }));
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  useBindAccount: () => hooks.bind,
  useDetachAccount: () => hooks.detach,
  usePersonAccounts: () => hooks.owned,
  useExcludeAccount: () => hooks.exclude,
  usePersonList: () => hooks.search,
}));

import { AccountActions } from "./account-actions";

const REF = {
  source: "github",
  source_id: "01900000-0000-7000-8000-00000000aa01",
  account_id: "dev-42",
};
const BOB = {
  person_id: "01900000-0000-7000-8000-0000000000b0",
  display_name: "Bob Park",
  email: "bob.park@example.com",
};
const CAROL = {
  person_id: "01900000-0000-7000-8000-0000000000c0",
  display_name: "Carol Chen",
  email: "carol.chen@example.com",
};

function binding(over: Partial<AccountBinding> = {}): AccountBinding {
  return { ...REF, person_id: BOB.person_id, history: [], ...over };
}

beforeEach(() => {
  for (const verb of [hooks.bind, hooks.detach, hooks.exclude]) {
    verb.mutate.mockClear();
    verb.reset.mockClear();
    verb.isError = false;
    verb.error = null;
    verb.isPending = false;
  }
  hooks.toast.success.mockClear();
  hooks.toast.error.mockClear();
  // Two accounts by default, so the detach verb is on offer in the cases that
  // are not about the detach gate.
  hooks.owned.data = { person_id: BOB.person_id, accounts: [{}, {}] };
  hooks.search.data = undefined;
});

describe("AccountActions", () => {
  it("labels the bound candidate Confirm and the rival Bind", () => {
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB, CAROL]} />,
    );

    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Bind" })).toBeInTheDocument();
  });

  // Candidates answer "whose COULD it be" — a question only an undecided account
  // has. Where the account is settled the surface sends none, and the window must
  // not invent one: re-asserting an existing binding is the CONFIRM act, and the
  // queue is where the accounts that need it are listed.
  it("offers nothing to confirm where the surface sends no candidates", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[]}
        holder={BOB}
      />,
    );

    expect(screen.queryByRole("button", { name: /^confirm$/i })).not.toBeInTheDocument();
    expect(screen.queryByText(/candidates/i)).not.toBeInTheDocument();
  });

  // This picker moves an account to somebody ELSE, so the one person it must not
  // offer is the one who already holds it — with or without a candidate list to
  // learn that from.
  it("keeps the holder out of the person search", async () => {
    hooks.search.data = { pages: [{ items: [BOB, CAROL] }] };
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[]}
        holder={BOB}
      />,
    );

    await userEvent.type(screen.getByRole("searchbox"), "park");

    expect(
      screen.getByRole("button", { name: /carol chen/i }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /bob park/i }),
    ).not.toBeInTheDocument();
  });

  // Opened from inside a person: they are the whole reason the reader is here,
  // so binding to them is one press and the search for a person is gone —
  // finding somebody you already have open is being asked twice.
  it("binds straight to the person the surface has open, with no search", async () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: null })}
        candidates={[]}
        bindTo={CAROL}
      />,
    );

    expect(screen.queryByRole("searchbox")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: /bind to selected person/i }),
    );
    await userEvent.click(screen.getByRole("button", { name: /^bind$/i }));

    expect(hooks.bind.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ person_id: CAROL.person_id }),
      expect.anything(),
    );
  });

  // A button that reads like a decision and changes nothing is worse than no
  // button: the account is already theirs, and the trail would gain a row
  // saying so twice.
  it("offers nothing to bind when the account is already that person's", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: CAROL.person_id })}
        candidates={[]}
        bindTo={CAROL}
      />,
    );

    expect(
      screen.queryByRole("button", { name: /bind to/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("searchbox")).not.toBeInTheDocument();
    // The other verbs are untouched — it is still an account under review.
    expect(screen.getByRole("button", { name: /detach/i })).toBeInTheDocument();
  });

  // Without a person behind the window the picker is the only way in, and the
  // accounts mode is entered with no person at all.
  it("keeps the person search where no person is open", () => {
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[]} />,
    );

    expect(screen.getByRole("searchbox")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /bind to/i }),
    ).not.toBeInTheDocument();
  });

  // A detach mints a person and moves the account there. Taking somebody's ONLY
  // account replaces them with an identical person and leaves their name behind
  // with nothing attached — which is how the accountless persons got made.
  it("offers no detach where it would leave the holder with nothing", () => {
    hooks.owned.data = { person_id: BOB.person_id, accounts: [{}] };
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[]}
        holder={BOB}
      />,
    );

    expect(
      screen.queryByRole("button", { name: /detach into a new person/i }),
    ).not.toBeInTheDocument();
    // The other verbs are unaffected.
    expect(screen.getByRole("button", { name: /exclude \(bot/i })).toBeInTheDocument();
  });

  it("offers a detach once the holder has more than one account", () => {
    hooks.owned.data = { person_id: BOB.person_id, accounts: [{}, {}] };
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[]}
        holder={BOB}
      />,
    );

    expect(
      screen.getByRole("button", { name: /detach into a new person/i }),
    ).toBeInTheDocument();
  });

  // An account nobody holds is the exception: a detach is how an orphan gets a
  // person of its own, and there is no holder to strip.
  it("offers a detach for an account nobody holds, whatever the read says", () => {
    hooks.owned.data = undefined;
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: null })}
        candidates={[]}
      />,
    );

    expect(
      screen.getByRole("button", { name: /detach into a new person/i }),
    ).toBeInTheDocument();
  });

  // Held back rather than shown and then withdrawn: offering a verb before
  // knowing it means anything is the mistake being fixed.
  it("holds the detach back while the holder's accounts are still unread", () => {
    hooks.owned.data = undefined;
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[]}
        holder={BOB}
      />,
    );

    expect(
      screen.queryByRole("button", { name: /detach into a new person/i }),
    ).not.toBeInTheDocument();
  });

  // The holder is named once, with the verb that acts on them. Listing them a
  // second time under "candidates" just to hang Confirm off it read as two
  // people with the same name and the same id.
  it("names the holder once, with Confirm on their own card", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[BOB]}
        holder={BOB}
      />,
    );

    expect(screen.getByText(/currently bound to/i)).toBeInTheDocument();
    expect(screen.getAllByText("Bob Park")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    // Nothing left to list: the only candidate was the holder.
    expect(screen.queryByText(/candidates/i)).not.toBeInTheDocument();
  });

  it("lists only the rivals under candidates, each offering a plain bind", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[BOB, CAROL]}
        holder={BOB}
      />,
    );

    expect(screen.getByText(/candidates/i)).toBeInTheDocument();
    expect(screen.getAllByText("Carol Chen")).toHaveLength(1);
    expect(screen.getAllByText("Bob Park")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Bind" })).toBeInTheDocument();
  });

  // A settled account is not queued for confirmation, so the surface sends no
  // candidates — and re-asserting a decision already taken changes nothing.
  it("offers no Confirm where the account is not up for confirmation", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[]}
        holder={BOB}
      />,
    );

    expect(screen.getByText(/currently bound to/i)).toBeInTheDocument();
    expect(screen.getByText("Bob Park")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Confirm" })).not.toBeInTheDocument();
  });

  it("keeps an unknown holder an honest bare id", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: "01900000-0000-7000-8000-00000000ffff" })}
        candidates={[]}
      />,
    );

    expect(
      screen.getByText("01900000-0000-7000-8000-00000000ffff"),
    ).toBeInTheDocument();
  });

  it("states an unbound account as a fact", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: null })}
        candidates={[]}
      />,
    );

    expect(screen.getByText(/account is unresolved/i)).toBeInTheDocument();
  });

  // A merge is a claim about PEOPLE, and this window argues about one account:
  // the button used to sit in the row of the person who would survive while the
  // absorbed one was named in a section above it. It belongs to the queue's case.
  it("never offers a merge, however many candidates there are", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[BOB, CAROL]}
      />,
    );

    expect(screen.queryByRole("button", { name: /merge/i })).not.toBeInTheDocument();
  });

  it("bind goes through a confirmation and sends the WIRE account shape", async () => {
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB, CAROL]} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Bind" }));
    const dialog = screen.getByRole("dialog");
    expect(hooks.bind.mutate).not.toHaveBeenCalled();

    await userEvent.click(within(dialog).getByRole("button", { name: "Bind" }));
    expect(hooks.bind.mutate).toHaveBeenCalledWith(
      {
        account: { source: REF.source, source_id: REF.source_id, id: REF.account_id },
        person_id: CAROL.person_id,
      },
      expect.anything(),
    );
  });

  // A confirmation an operator can consent to has to say what changes.
  // Re-asserting the binding in force moves nothing — the description that
  // suits a bind ("re-bound to this person") states the opposite.
  it("tells a confirm apart from a bind in what it promises", async () => {
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB, CAROL]} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(
      within(screen.getByRole("dialog")).getByText(/stays bound to the current person/i),
    ).toBeInTheDocument();

    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Bind" }));
    expect(
      within(screen.getByRole("dialog")).getByText(/re-bound to this person/i),
    ).toBeInTheDocument();
  });

  it("renders the server's outcome verbatim, refusals included", async () => {
    const refusal: CorrectionResponse = {
      applied: 0,
      already_decided: 0,
      items: [{ ...REF, account_id: REF.account_id, outcome: "refused" }],
    };
    hooks.detach.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.(refusal),
    );
    const onDecided = vi.fn();
    render(
      <AccountActions
        accountRef={REF}
        binding={binding()}
        candidates={[]}
        onDecided={onDecided}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: /detach into a new person/i }),
    );
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Detach" }),
    );

    expect(screen.getByText(/1 refused/i)).toBeInTheDocument();
    expect(screen.getByText(/0 applied/i)).toBeInTheDocument();
    // The account kept its binding: there is still something here to decide.
    expect(onDecided).not.toHaveBeenCalled();
    expect(hooks.toast.error).toHaveBeenCalledOnce();
  });

  // The whole point of handing the window back: what it shows — the candidate
  // list, the binding, the verbs — is a read the server has just moved past.
  // Left open, the same Merge is one click away and fires a second time.
  it("hands the window back when every account it named was decided", async () => {
    const applied: CorrectionResponse = {
      applied: 1,
      already_decided: 0,
      items: [{ ...REF, account_id: REF.account_id, outcome: "applied" }],
    };
    hooks.bind.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.(applied),
    );
    const onDecided = vi.fn();
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[BOB, CAROL]}
        onDecided={onDecided}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Bind" }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Bind" }),
    );

    expect(onDecided).toHaveBeenCalledOnce();
    expect(hooks.toast.success).toHaveBeenCalledOnce();
    // Counters belong to the window that stays; a clean result reports by toast.
    expect(screen.queryByText(/1 applied/i)).not.toBeInTheDocument();
  });

  // `already_decided` is not a failure — somebody got there first — so the
  // window still goes back, and the message says nothing changed.
  it("hands the window back on already-decided too, saying nothing changed", async () => {
    const already: CorrectionResponse = {
      applied: 0,
      already_decided: 1,
      items: [
        { ...REF, account_id: REF.account_id, outcome: "already_decided" },
      ],
    };
    hooks.exclude.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.(already),
    );
    const onDecided = vi.fn();
    render(
      <AccountActions
        accountRef={REF}
        binding={binding()}
        candidates={[]}
        onDecided={onDecided}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: /exclude \(bot/i }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Exclude" }),
    );

    expect(onDecided).toHaveBeenCalledOnce();
    expect(hooks.toast.success).toHaveBeenCalledWith(
      "Already decided — nothing changed.",
      expect.anything(),
    );
  });

  // The confirmation over these buttons is modal, so it already blocks them.
  // This covers the frame between the write landing and the window unmounting.
  it("locks every verb while one is in flight", () => {
    hooks.bind.isPending = true;
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[BOB, CAROL]}
      />,
    );

    for (const name of [
      "Confirm",
      "Bind",
      /detach into a new person/i,
      /exclude \(bot/i,
    ]) {
      expect(screen.getByRole("button", { name })).toBeDisabled();
    }
  });

  // The same window opens over settled accounts from the search and from a
  // person's own list. Promising those the queue describes a queue they were
  // never in.
  it("promises the queue only for an account that is on it", async () => {
    const { rerender } = render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: null })}
        candidates={[]}
        bindTo={CAROL}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: /bind to selected person/i }),
    );
    expect(
      within(screen.getByRole("dialog")).queryByText(/leaves the queue/i),
    ).not.toBeInTheDocument();
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );

    rerender(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: null })}
        candidates={[]}
        bindTo={CAROL}
        queued
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: /bind to selected person/i }),
    );
    expect(
      within(screen.getByRole("dialog")).getByText(/leaves the queue/i),
    ).toBeInTheDocument();
  });

  // An unbound account is not being RE-bound: the commonest queue case is an
  // account nobody holds, and telling the operator otherwise describes a
  // binding that does not exist.
  it("says bound for an account nobody holds and re-bound for one that is held", async () => {
    const { rerender } = render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: null })}
        candidates={[]}
        bindTo={CAROL}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: /bind to selected person/i }),
    );
    expect(
      within(screen.getByRole("dialog")).getByText(
        /^The account is bound to this person\.$/,
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );

    rerender(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[]}
        bindTo={CAROL}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: /bind to selected person/i }),
    );
    expect(
      within(screen.getByRole("dialog")).getByText(/is re-bound to this person/i),
    ).toBeInTheDocument();
  });

  // The counters belong to the attempt that produced them. Today `onDecided`
  // unmounts this window, but the prop is optional and a caller may keep it.
  it("clears a previous attempt's counters when the next one succeeds", async () => {
    const refusal: CorrectionResponse = {
      applied: 0,
      already_decided: 0,
      items: [{ ...REF, account_id: REF.account_id, outcome: "refused" }],
    };
    hooks.exclude.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.(refusal),
    );
    hooks.detach.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: CorrectionResponse) => void }) =>
        opts?.onSuccess?.({
          applied: 1,
          already_decided: 0,
          items: [{ ...REF, account_id: REF.account_id, outcome: "applied" }],
        }),
    );
    render(<AccountActions accountRef={REF} binding={binding()} candidates={[]} />);

    await userEvent.click(screen.getByRole("button", { name: /exclude \(bot/i }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Exclude" }),
    );
    expect(screen.getByText(/1 refused/i)).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: /detach into a new person/i }),
    );
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Detach" }),
    );

    expect(screen.queryByText(/1 refused/i)).not.toBeInTheDocument();
  });

  it("cancelling a dialog resets the verbs, so the next one opens clean", async () => {
    render(<AccountActions accountRef={REF} binding={binding()} candidates={[]} />);

    await userEvent.click(screen.getByRole("button", { name: /exclude \(bot/i }));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }),
    );

    // A dialog's error belongs to the attempt made in that dialog; without
    // the reset the next dialog would open already wearing the old failure.
    expect(hooks.exclude.reset).toHaveBeenCalled();
    expect(hooks.bind.reset).toHaveBeenCalled();
  });

  it("a failed verb keeps the dialog open with the server's reason", async () => {
    hooks.exclude.isError = true;
    const { IdentityApiError } = await import("@/api/identity-client");
    hooks.exclude.error = new IdentityApiError(400, {
      context: { field_violations: [{ field: "account", description: "account not found" }] },
    });
    render(<AccountActions accountRef={REF} binding={binding()} candidates={[]} />);

    await userEvent.click(screen.getByRole("button", { name: /exclude \(bot/i }));
    const dialog = screen.getByRole("dialog");

    expect(within(dialog).getByText("account not found")).toBeInTheDocument();
  });
});
