// @vitest-environment jsdom
/**
 * Confirming a whole queue group. What matters: which groups it is offered for
 * at all, that it sends one binding per row aimed at the person that row already
 * has, that a comment is required before it can be pressed, that a group larger
 * than the endpoint's cap is split into several calls and folded into one answer,
 * and that a failure is reported by toast as well as in the dialog.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AttentionItem, CorrectionResponse } from "@/api/identity-client";

const hooks = vi.hoisted(() => ({
  toast: { success: vi.fn(), error: vi.fn() },
  bind: { mutateAsync: vi.fn(), reset: vi.fn() },
}));
vi.mock("@/components/ui/sonner", () => ({ toast: hooks.toast }));
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  useBindAccounts: () => hooks.bind,
}));

import { ConfirmGroupButton } from "./confirm-group-button";
import { groupIsConfirmable } from "@/lib/identities/cases";

const SOURCE_ID = "01900000-0000-7000-8000-00000000aa01";

function item(over: Partial<AttentionItem> = {}): AttentionItem {
  return {
    kind: "minted_from_roster",
    source: "github",
    source_id: SOURCE_ID,
    account_id: "dev-42",
    bound_to: "01900000-0000-7000-8000-0000000000b0",
    candidates: [],
    ...over,
  };
}

function applied(count: number): CorrectionResponse {
  return {
    applied: count,
    already_decided: 0,
    items: Array.from({ length: count }, (_, i) => ({
      source: "github",
      source_id: SOURCE_ID,
      account_id: `dev-${i}`,
      outcome: "applied",
    })),
  };
}

const press = () => screen.getByRole("button", { name: /confirm all|confirm 1/i });
const confirmIn = (dialog: HTMLElement) =>
  within(dialog).getByRole("button", { name: "Confirm" });

beforeEach(() => {
  hooks.toast.success.mockClear();
  hooks.toast.error.mockClear();
  hooks.bind.reset.mockClear();
  hooks.bind.mutateAsync.mockReset();
  hooks.bind.mutateAsync.mockResolvedValue(applied(1));
});

describe("groupIsConfirmable", () => {
  const cases: [string, string, boolean][] = [
    ["a login mint", "provisioned_at_login", true],
    ["a roster mint", "minted_from_roster", true],
    // No single answer to apply, nothing to ratify, nothing bound.
    ["a contested account", "contested", false],
    ["a binding conflict", "binding_conflict", false],
    ["an account with no evidence", "no_evidence", false],
  ];

  for (const [name, kind, expected] of cases) {
    it(`${expected ? "offers" : "refuses"} a group of ${name}`, () => {
      expect(groupIsConfirmable(kind, [item({ kind })])).toBe(expected);
    });
  }

  it("refuses a group where any row names no person to confirm", () => {
    expect(
      groupIsConfirmable("minted_from_roster", [
        item({ account_id: "a1" }),
        item({ account_id: "a2", bound_to: null }),
      ]),
    ).toBe(false);
  });

  it("refuses an empty group", () => {
    expect(groupIsConfirmable("minted_from_roster", [])).toBe(false);
  });
});

describe("ConfirmGroupButton", () => {
  it("names how many accounts one press would settle", () => {
    render(
      <ConfirmGroupButton
        items={[item({ account_id: "a1" }), item({ account_id: "a2" })]}
      />,
    );

    expect(
      screen.getByRole("button", { name: "Confirm all 2 accounts" }),
    ).toBeInTheDocument();
  });

  // One press stands in for a hundred decisions; the trail has to say on what
  // grounds, so the comment is required rather than optional.
  it("will not fire without a comment", async () => {
    render(<ConfirmGroupButton items={[item()]} />);

    await userEvent.click(press());
    const dialog = screen.getByRole("dialog");
    expect(confirmIn(dialog)).toBeDisabled();

    await userEvent.type(within(dialog).getByRole("textbox"), "checked with HR");
    expect(confirmIn(dialog)).toBeEnabled();
  });

  it("sends one binding per row, each to the person that row already has", async () => {
    render(
      <ConfirmGroupButton
        items={[
          item({ account_id: "a1", bound_to: "p-1" }),
          item({ account_id: "a2", bound_to: "p-2" }),
        ]}
      />,
    );

    await userEvent.click(press());
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox"), "roster is right");
    await userEvent.click(confirmIn(dialog));

    expect(hooks.bind.mutateAsync).toHaveBeenCalledOnce();
    expect(hooks.bind.mutateAsync).toHaveBeenCalledWith({
      bindings: [
        {
          account: { source: "github", source_id: SOURCE_ID, id: "a1" },
          person_id: "p-1",
        },
        {
          account: { source: "github", source_id: SOURCE_ID, id: "a2" },
          person_id: "p-2",
        },
      ],
      comment: "roster is right",
    });
    expect(hooks.toast.success).toHaveBeenCalledOnce();
  });

  // The endpoint caps a call at 1000 bindings. A bigger group is several calls
  // and ONE answer — an operator took one decision.
  it("splits a group past the endpoint's cap into several calls", async () => {
    const items = Array.from({ length: 1001 }, (_, i) =>
      item({ account_id: `a${i}` }),
    );
    hooks.bind.mutateAsync
      .mockResolvedValueOnce(applied(1000))
      .mockResolvedValueOnce(applied(1));
    render(<ConfirmGroupButton items={items} />);

    await userEvent.click(press());
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox"), "bulk");
    await userEvent.click(confirmIn(dialog));

    expect(hooks.bind.mutateAsync).toHaveBeenCalledTimes(2);
    expect(hooks.bind.mutateAsync.mock.calls[0][0].bindings).toHaveLength(1000);
    expect(hooks.bind.mutateAsync.mock.calls[1][0].bindings).toHaveLength(1);
    expect(hooks.toast.success).toHaveBeenCalledWith("Done — 1001 accounts updated.");
  });

  // Rows are pruned as each call lands, so this dialog can be unmounted before a
  // later failure could be read in it.
  it("reports a failure by toast as well as in the dialog", async () => {
    const { IdentityApiError } = await import("@/api/identity-client");
    hooks.bind.mutateAsync.mockRejectedValueOnce(
      new IdentityApiError(400, {
        context: {
          field_violations: [{ field: "bindings", description: "person not found" }],
        },
      }),
    );
    render(<ConfirmGroupButton items={[item()]} />);

    await userEvent.click(press());
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox"), "why not");
    await userEvent.click(confirmIn(dialog));

    expect(within(screen.getByRole("dialog")).getByText("person not found")).toBeInTheDocument();
    expect(hooks.toast.error).toHaveBeenCalledWith("person not found");
    expect(hooks.toast.success).not.toHaveBeenCalled();
  });

  it("keeps the dialog open when the server refused an account", async () => {
    hooks.bind.mutateAsync.mockResolvedValue({
      applied: 0,
      already_decided: 0,
      items: [
        { source: "github", source_id: SOURCE_ID, account_id: "a1", outcome: "refused" },
      ],
    } satisfies CorrectionResponse);
    render(<ConfirmGroupButton items={[item()]} />);

    await userEvent.click(press());
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox"), "try");
    await userEvent.click(confirmIn(dialog));

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(hooks.toast.error).toHaveBeenCalledOnce();
  });
});
