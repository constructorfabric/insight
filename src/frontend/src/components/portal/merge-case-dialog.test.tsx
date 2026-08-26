// @vitest-environment jsdom
/**
 * Merging a case. What matters: both sides are named before anything happens;
 * the preview of what moves is the consent, so its COUNT is what the dialog
 * shows and confirm is locked until every read lands (and locked for good when
 * nothing would move); the absorbed set is chosen, so unticking a person keeps
 * their accounts where they are; a case of three or more issues one call per
 * ticked person, all aimed at the survivor, and STOPS at the first failure; and
 * every failure is reported by toast as well as in the dialog, because the first
 * successful merge can unmount the dialog before the second is answered.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { CorrectionResponse } from "@/api/identity-client";

const hooks = vi.hoisted(() => ({
  toast: { success: vi.fn(), error: vi.fn() },
  merge: { mutateAsync: vi.fn() },
  moving: {
    ready: true,
    failed: false,
    accounts: [] as unknown[],
    refetch: vi.fn(),
  },
  /** Person ids the dialog asked the preview for, per render. */
  asked: [] as string[][],
}));
vi.mock("@/components/ui/sonner", () => ({ toast: hooks.toast }));
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  useMergePersons: () => hooks.merge,
  usePersonAccountsMany: (ids: string[]) => {
    hooks.asked.push(ids);
    return hooks.moving;
  },
}));

import { MergeCaseDialog } from "./merge-case-dialog";

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
const DAN = {
  person_id: "01900000-0000-7000-8000-0000000000d0",
  display_name: "Dan Ito",
  email: "dan.ito@example.com",
};

const ACCOUNT = {
  source: "github",
  source_id: "01900000-0000-7000-8000-00000000aa01",
  account_id: "dev-42",
  email: "dev42@example.com",
};

function account(id: string, source = "github") {
  return { ...ACCOUNT, source, account_id: id, email: `${id}@example.com` };
}

function applied(count: number): CorrectionResponse {
  return {
    applied: count,
    already_decided: 0,
    items: Array.from({ length: count }, (_, i) => ({
      ...account(`moved-${i}`),
      outcome: "applied",
    })),
  };
}

const confirm = () => screen.getByRole("button", { name: "Merge persons" });
const lastAsked = () => hooks.asked.at(-1) ?? [];

beforeEach(() => {
  hooks.toast.success.mockClear();
  hooks.toast.error.mockClear();
  hooks.moving.refetch.mockClear();
  hooks.moving.ready = true;
  hooks.moving.failed = false;
  hooks.moving.accounts = [ACCOUNT];
  hooks.merge.mutateAsync.mockReset();
  hooks.merge.mutateAsync.mockResolvedValue(applied(1));
  hooks.asked.length = 0;
});

describe("MergeCaseDialog", () => {
  it("names who stays above who is absorbed", () => {
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={vi.fn()} />,
    );

    const dialog = screen.getByRole("dialog");
    expect(
      within(dialog).getByText(/merge the rest into bob park/i),
    ).toBeInTheDocument();
    // Order carries the meaning: the first person named is the one that remains.
    const labels = within(dialog)
      .getAllByText(/^(stays|merged into them)$/i)
      .map((el) => el.textContent?.toLowerCase());
    expect(labels).toEqual(["stays", "merged into them"]);
    const names = within(dialog)
      .getAllByText(/^(Bob Park|Carol Chen)$/)
      .map((el) => el.textContent);
    expect(names).toEqual(["Bob Park", "Carol Chen"]);
  });

  it("counts the people it would absorb once there is more than one", () => {
    render(
      <MergeCaseDialog
        survivor={BOB}
        absorbed={[CAROL, DAN]}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByText(/merged into them — 2 people/i)).toBeInTheDocument();
  });

  // The preview IS the consent. Showing a count the reads do not support is the
  // exact failure the confirm gate exists to prevent.
  it("shows how many accounts move, and names them", () => {
    hooks.moving.accounts = [account("a1"), account("a2", "gitlab")];
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={vi.fn()} />,
    );

    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("2 accounts move:")).toBeInTheDocument();
    expect(within(dialog).getByText(/github · a1@example\.com/)).toBeInTheDocument();
    expect(within(dialog).getByText(/gitlab · a2@example\.com/)).toBeInTheDocument();
  });

  it("names the first five accounts and says how many more there are", () => {
    hooks.moving.accounts = Array.from({ length: 7 }, (_, i) => account(`a${i}`));
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={vi.fn()} />,
    );

    expect(screen.getByText("7 accounts move:")).toBeInTheDocument();
    expect(screen.getByText(/github · a4@example\.com/)).toBeInTheDocument();
    expect(screen.queryByText(/github · a5@example\.com/)).not.toBeInTheDocument();
    expect(screen.getByText(/and 2 more/i)).toBeInTheDocument();
  });

  it("locks confirm until every read of what moves has landed", () => {
    hooks.moving.ready = false;
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={vi.fn()} />,
    );

    expect(confirm()).toBeDisabled();
    expect(screen.getByText(/counting the accounts/i)).toBeInTheDocument();
  });

  // A merge that moves nothing is not a decision: the server answers "0 applied,
  // 0 already decided" with no error, and the case comes straight back.
  it("locks confirm when nothing would move at all", () => {
    hooks.moving.accounts = [];
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={vi.fn()} />,
    );

    expect(confirm()).toBeDisabled();
    expect(screen.getByText("0 accounts move:")).toBeInTheDocument();
  });

  it("a failed preview blocks the merge and offers a retry, not a zero-account lie", async () => {
    hooks.moving.failed = true;
    hooks.moving.ready = false;
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={vi.fn()} />,
    );

    expect(confirm()).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(hooks.moving.refetch).toHaveBeenCalledOnce();
  });

  // Two of a case's three people can be one human while the third is a different
  // one who happens to share an address. All-or-nothing would rebind their
  // accounts irreversibly.
  it("leaves an unticked person's accounts where they are", async () => {
    const onClose = vi.fn();
    render(
      <MergeCaseDialog
        survivor={BOB}
        absorbed={[CAROL, DAN]}
        onClose={onClose}
      />,
    );

    await userEvent.click(screen.getByRole("checkbox", { name: /dan ito/i }));
    // The preview follows the choice, or it would consent to the wrong set.
    expect(lastAsked()).toEqual([CAROL.person_id]);

    await userEvent.click(confirm());

    expect(hooks.merge.mutateAsync).toHaveBeenCalledOnce();
    expect(hooks.merge.mutateAsync).toHaveBeenCalledWith({
      source_person_id: CAROL.person_id,
      target_person_id: BOB.person_id,
    });
  });

  it("has nothing to confirm once every person is unticked", async () => {
    render(
      <MergeCaseDialog
        survivor={BOB}
        absorbed={[CAROL, DAN]}
        onClose={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("checkbox", { name: /carol chen/i }));
    await userEvent.click(screen.getByRole("checkbox", { name: /dan ito/i }));

    expect(confirm()).toBeDisabled();
    expect(screen.getByText(/tick at least one person/i)).toBeInTheDocument();
  });

  // The endpoint joins exactly two persons, so the survivor absorbs the rest one
  // call at a time — every one of them aimed at the same target.
  it("issues one call per ticked person, all into the survivor", async () => {
    const onClose = vi.fn();
    render(
      <MergeCaseDialog
        survivor={BOB}
        absorbed={[CAROL, DAN]}
        onClose={onClose}
      />,
    );

    await userEvent.click(confirm());

    expect(hooks.merge.mutateAsync).toHaveBeenCalledTimes(2);
    expect(hooks.merge.mutateAsync).toHaveBeenNthCalledWith(1, {
      source_person_id: CAROL.person_id,
      target_person_id: BOB.person_id,
    });
    expect(hooks.merge.mutateAsync).toHaveBeenNthCalledWith(2, {
      source_person_id: DAN.person_id,
      target_person_id: BOB.person_id,
    });
    // One decision, one answer: the counts are folded across the calls.
    expect(hooks.toast.success).toHaveBeenCalledWith("Done — 2 accounts updated.");
    expect(onClose).toHaveBeenCalledOnce();
  });

  // The failure is on the MIDDLE call, so "stops" is a claim the test can fail:
  // a loop that carried on would reach the third person.
  it("stops at the first failure instead of carrying on down the list", async () => {
    const { IdentityApiError } = await import("@/api/identity-client");
    hooks.merge.mutateAsync
      .mockResolvedValueOnce(applied(1))
      .mockRejectedValueOnce(
        new IdentityApiError(400, {
          context: {
            field_violations: [
              { field: "source_person_id", description: "person not found" },
            ],
          },
        }),
      )
      .mockResolvedValueOnce(applied(1));
    const onClose = vi.fn();
    const EVE = {
      person_id: "01900000-0000-7000-8000-0000000000e0",
      display_name: "Eve Ng",
    };
    render(
      <MergeCaseDialog
        survivor={BOB}
        absorbed={[CAROL, DAN, EVE]}
        onClose={onClose}
      />,
    );

    await userEvent.click(confirm());

    expect(hooks.merge.mutateAsync).toHaveBeenCalledTimes(2);
    expect(hooks.merge.mutateAsync).not.toHaveBeenCalledWith({
      source_person_id: EVE.person_id,
      target_person_id: BOB.person_id,
    });
    expect(onClose).not.toHaveBeenCalled();
    expect(hooks.toast.success).not.toHaveBeenCalled();
  });

  // The dialog's own error slot may be gone by the time it is written to: the
  // first successful merge prunes the rows it decided, re-keying the case and
  // remounting the block that owns this dialog. A toast outlives that.
  it("reports a mid-sequence failure by toast, naming what did move", async () => {
    const { IdentityApiError } = await import("@/api/identity-client");
    hooks.merge.mutateAsync
      .mockResolvedValueOnce(applied(2))
      .mockRejectedValueOnce(
        new IdentityApiError(400, {
          context: {
            field_violations: [
              { field: "source_person_id", description: "person not found" },
            ],
          },
        }),
      );
    render(
      <MergeCaseDialog
        survivor={BOB}
        absorbed={[CAROL, DAN]}
        onClose={vi.fn()}
      />,
    );

    await userEvent.click(confirm());

    expect(screen.getByText("person not found")).toBeInTheDocument();
    expect(hooks.toast.error).toHaveBeenCalledWith("person not found", {
      description: "2 accounts had already moved.",
    });
  });

  // A refusal means the accounts kept their bindings: the case is not settled,
  // so the dialog stays — and the toast says so even if the dialog is gone.
  it("reports a refusal by toast and keeps the dialog open", async () => {
    hooks.merge.mutateAsync.mockResolvedValue({
      applied: 0,
      already_decided: 0,
      items: [{ ...ACCOUNT, outcome: "refused" }],
    } satisfies CorrectionResponse);
    const onClose = vi.fn();
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={onClose} />,
    );

    await userEvent.click(confirm());

    expect(screen.getByText(/1 account was refused/i)).toBeInTheDocument();
    expect(hooks.toast.error).toHaveBeenCalledOnce();
    expect(onClose).not.toHaveBeenCalled();
  });

  // The outcome field is an open vocabulary. A value this build has never heard
  // of is not success, however few refusals came with it.
  it("does not close over an outcome it does not recognise", async () => {
    hooks.merge.mutateAsync.mockResolvedValue({
      applied: 1,
      already_decided: 0,
      items: [
        { ...account("a1"), outcome: "applied" },
        { ...account("a2"), outcome: "ambiguous_value" },
      ],
    } satisfies CorrectionResponse);
    const onClose = vi.fn();
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={onClose} />,
    );

    await userEvent.click(confirm());

    expect(onClose).not.toHaveBeenCalled();
    expect(hooks.toast.success).not.toHaveBeenCalled();
    expect(hooks.toast.error).toHaveBeenCalledOnce();
  });

  // The panel is a grid, and a grid item will not shrink below its own
  // content. An address and a person id are exactly the values that do not
  // wrap, so without this the body pushes itself out through the side —
  // invisible in jsdom, in a review, and to every other test here.
  it("lays the panel out so a value too wide for it truncates instead of escaping", () => {
    render(
      <MergeCaseDialog survivor={BOB} absorbed={[CAROL]} onClose={vi.fn()} />,
    );

    expect(screen.getByRole("dialog")).toHaveClass("[&>*]:min-w-0");
  });

});
