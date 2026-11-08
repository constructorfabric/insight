// @vitest-environment jsdom
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { CorrectionResponse } from "@/api/identity-client";
import { accountKey } from "@/lib/identities/account-key";

const hooks = vi.hoisted(() => {
  const verb = () => ({
    mutate: vi.fn(),
    reset: vi.fn(),
    isPending: false,
    isError: false,
    error: null as unknown,
  });
  return {
    toast: { add: vi.fn() },
    exclude: verb(),
    bind: verb(),
    detach: verb(),
    binding: {
      data: {
        source: "github",
        source_id: "01900000-0000-7000-8000-00000000aa01",
        account_id: "a1",
        person_id: null,
        history: [],
      } as unknown,
      isLoading: false,
      isError: false,
      error: null as unknown,
      refetch: vi.fn(),
    },
  };
});
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));
vi.mock("@gears-frontx/ui-kit", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@gears-frontx/ui-kit")>()),
  toast: hooks.toast,
}));
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  useAccountBinding: () => hooks.binding,
  useBindAccount: () => hooks.bind,
  useDetachAccount: () => hooks.detach,
  useExcludeAccount: () => hooks.exclude,
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

import { CaseDialog, type CaseRow } from "./case-dialog";

const SOURCE_ID = "01900000-0000-7000-8000-00000000aa01";

function row(account_id: string, kind = "no_evidence"): CaseRow {
  return {
    kind,
    source: "github",
    source_id: SOURCE_ID,
    account_id,
    email: `${account_id}@example.com`,
    username: null,
    candidates: [],
  };
}

const keyOf = (account_id: string) =>
  accountKey({ source: "github", source_id: SOURCE_ID, account_id });

function outcome(
  kind: "applied" | "refused",
  account_id: string
): CorrectionResponse {
  return {
    applied: kind === "applied" ? 1 : 0,
    already_decided: 0,
    items: [
      { source: "github", source_id: SOURCE_ID, account_id, outcome: kind },
    ],
  };
}

function decideWith(kind: "applied" | "refused") {
  hooks.exclude.mutate.mockImplementation(
    (
      args: { account: { id: string } },
      opts?: { onSuccess?: (r: CorrectionResponse) => void }
    ) => opts?.onSuccess?.(outcome(kind, args.account.id))
  );
}

function Harness({
  start,
  walk,
  rows,
}: {
  start: string;
  walk?: string[];
  rows: CaseRow[];
}) {
  const [acct, setAcct] = useState<string | undefined>(start);
  const [live, setLive] = useState(walk);
  return (
    <>
      <button
        type="button"
        onClick={() => setLive((now) => now?.filter((key) => key !== acct))}
      >
        prune
      </button>
      <CaseDialog
        acct={acct}
        items={rows}
        walk={live}
        onNavigate={setAcct}
        onClose={() => setAcct(undefined)}
      />
      <output data-testid="open">{acct ?? "closed"}</output>
    </>
  );
}

async function exclude() {
  await userEvent.click(screen.getByRole("button", { name: /exclude \(bot/i }));
  await userEvent.click(
    within(screen.getAllByRole("dialog").at(-1)!).getByRole("button", {
      name: "Exclude",
    })
  );
}

const ROWS = [row("a1"), row("a2"), row("a3")];
const WALK = ROWS.map((r) => keyOf(r.account_id));

beforeEach(() => {
  hooks.exclude.mutate.mockReset();
  hooks.toast.add.mockClear();
});

describe("CaseDialog walk", () => {
  it("opens the next account after a decision", async () => {
    decideWith("applied");
    render(<Harness start={WALK[0]} walk={WALK} rows={ROWS} />);

    await exclude();

    expect(screen.getByTestId("open")).toHaveTextContent(WALK[1]);
  });

  it("walks the list as it stood when the account opened, though decided rows are pruned underneath", async () => {
    decideWith("applied");
    render(<Harness start={WALK[0]} walk={WALK} rows={ROWS} />);

    fireEvent.click(
      screen.getByRole("button", { name: "prune", hidden: true })
    );
    await exclude();

    expect(screen.getByTestId("open")).toHaveTextContent(WALK[1]);
  });

  it("closes after a decision on the last account", async () => {
    decideWith("applied");
    render(<Harness start={WALK[2]} walk={WALK} rows={ROWS} />);

    await exclude();

    expect(screen.getByTestId("open")).toHaveTextContent("closed");
  });

  it("moves on without a decision when Skip is pressed", async () => {
    render(<Harness start={WALK[0]} walk={WALK} rows={ROWS} />);

    await userEvent.click(screen.getByRole("button", { name: "Skip" }));
    expect(screen.getByTestId("open")).toHaveTextContent(WALK[1]);
    await userEvent.click(screen.getByRole("button", { name: "Skip" }));

    expect(screen.getByTestId("open")).toHaveTextContent(WALK[2]);
    expect(hooks.exclude.mutate).not.toHaveBeenCalled();
  });

  it("disables Skip on the last account", () => {
    render(<Harness start={WALK[2]} walk={WALK} rows={ROWS} />);

    expect(screen.getByRole("button", { name: "Skip" })).toBeDisabled();
  });

  it("stays open on the same account when the outcome is refused", async () => {
    decideWith("refused");
    render(<Harness start={WALK[0]} walk={WALK} rows={ROWS} />);

    await exclude();

    expect(screen.getByTestId("open")).toHaveTextContent(WALK[0]);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("closes after a decision on an account outside the walk and shows no Skip", async () => {
    decideWith("applied");
    const outside = row("zz");
    render(
      <Harness start={keyOf("zz")} walk={WALK} rows={[...ROWS, outside]} />
    );

    expect(
      screen.queryByRole("button", { name: "Skip" })
    ).not.toBeInTheDocument();
    await exclude();

    expect(screen.getByTestId("open")).toHaveTextContent("closed");
  });

  it("closes on a decision and shows no Skip when no walk is passed", async () => {
    decideWith("applied");
    render(<Harness start={WALK[0]} rows={ROWS} />);

    expect(
      screen.queryByRole("button", { name: "Skip" })
    ).not.toBeInTheDocument();
    await exclude();

    expect(screen.getByTestId("open")).toHaveTextContent("closed");
  });
});
