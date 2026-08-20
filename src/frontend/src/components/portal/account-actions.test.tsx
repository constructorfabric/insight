// @vitest-environment jsdom
/**
 * The decision surface. What matters: the button on the currently bound
 * candidate reads "Confirm" (re-asserting IS the decision) while others read
 * "Bind"; Merge exists only when there is someone to absorb; every verb goes
 * through a confirmation and sends the account in the WIRE shape (`id`, not
 * `account_id`); the merge dialog previews what moves BEFORE anything
 * happens; and the server's outcome renders verbatim — a refusal is never
 * dressed up as success.
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
    bind: verb(),
    merge: verb(),
    detach: verb(),
    exclude: verb(),
    personAccounts: {
      data: undefined as
        | { person_id: string; accounts: unknown[] }
        | undefined,
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    },
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
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  useBindAccount: () => hooks.bind,
  useMergePersons: () => hooks.merge,
  useDetachAccount: () => hooks.detach,
  useExcludeAccount: () => hooks.exclude,
  usePersonAccounts: () => hooks.personAccounts,
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
  for (const verb of [hooks.bind, hooks.merge, hooks.detach, hooks.exclude]) {
    verb.mutate.mockClear();
    verb.reset.mockClear();
    verb.isError = false;
    verb.error = null;
  }
  hooks.search.data = undefined;
  hooks.personAccounts.data = undefined;
  hooks.personAccounts.isLoading = false;
  hooks.personAccounts.isError = false;
  hooks.personAccounts.refetch.mockClear();
});

describe("AccountActions", () => {
  it("labels the bound candidate Confirm and the rival Bind; Merge only on the rival", () => {
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB, CAROL]} />,
    );

    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Bind" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Merge…" })).toHaveLength(1);
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

  // The holder's card used to reach this component only as a candidate, so the
  // copy that names them broke the moment the candidate went away.
  it("still names the holder in the detach copy with no candidates", async () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: BOB.person_id })}
        candidates={[]}
        holder={BOB}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: /detach/i }));

    expect(
      screen.getByText(/stops counting towards Bob Park/i),
    ).toBeInTheDocument();
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

  it("offers no Merge when the account is bound to nobody", () => {
    render(
      <AccountActions
        accountRef={REF}
        binding={binding({ person_id: null })}
        candidates={[BOB]}
      />,
    );

    expect(screen.queryByRole("button", { name: "Merge…" })).not.toBeInTheDocument();
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
  // Re-asserting the binding in force changes no binding at all — the
  // description that suits a bind ("the account moves to X") states the
  // opposite of what happens here.
  it("tells a confirm apart from a bind in what it promises", async () => {
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB, CAROL]} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(
      within(screen.getByRole("dialog")).getByText(/binding does not change/i),
    ).toBeInTheDocument();
  });

  it("names who a detach takes the account away from", async () => {
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB]} />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: /detach into a new person/i }),
    );
    expect(
      within(screen.getByRole("dialog")).getByText(/stops counting towards Bob Park/i),
    ).toBeInTheDocument();
  });

  it("the merge dialog previews what moves before anything happens", async () => {
    hooks.personAccounts.data = {
      person_id: BOB.person_id,
      accounts: [
        { source: "github", source_id: "s", account_id: "gh-1", email: "a@example.com" },
        { source: "gitlab", source_id: "s", account_id: "gl-2", username: "gl-2" },
      ],
    };
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB, CAROL]} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Merge…" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText(/2 accounts move/i)).toBeInTheDocument();
    expect(hooks.merge.mutate).not.toHaveBeenCalled();

    await userEvent.click(
      within(dialog).getByRole("button", { name: "Merge persons" }),
    );
    expect(hooks.merge.mutate).toHaveBeenCalledWith(
      { source_person_id: BOB.person_id, target_person_id: CAROL.person_id },
      expect.anything(),
    );
  });

  it("locks merge confirm until the preview names what moves", async () => {
    hooks.personAccounts.isLoading = true;
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB, CAROL]} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Merge…" }));
    const dialog = screen.getByRole("dialog");

    // The preview IS the consent — confirming a list the operator never saw
    // would move accounts sight-unseen.
    expect(
      within(dialog).getByRole("button", { name: "Merge persons" }),
    ).toBeDisabled();
    expect(hooks.merge.mutate).not.toHaveBeenCalled();
  });

  it("a failed preview blocks merge and offers a retry, not a zero-account lie", async () => {
    hooks.personAccounts.isError = true;
    render(
      <AccountActions accountRef={REF} binding={binding()} candidates={[BOB, CAROL]} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Merge…" }));
    const dialog = screen.getByRole("dialog");

    expect(within(dialog).queryByText(/0 accounts move/i)).not.toBeInTheDocument();
    expect(within(dialog).getByText(/could not count/i)).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Merge persons" }),
    ).toBeDisabled();

    await userEvent.click(within(dialog).getByRole("button", { name: "Retry" }));
    expect(hooks.personAccounts.refetch).toHaveBeenCalledOnce();
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
    render(<AccountActions accountRef={REF} binding={binding()} candidates={[]} />);

    await userEvent.click(
      screen.getByRole("button", { name: /detach into a new person/i }),
    );
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Detach" }),
    );

    expect(screen.getByText(/1 refused/i)).toBeInTheDocument();
    expect(screen.getByText(/0 applied/i)).toBeInTheDocument();
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
