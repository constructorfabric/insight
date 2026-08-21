/**
 * The account picker. What matters: it is a FINDING tool — the click hands the
 * account back and never writes; a blank field asks nothing, because the whole
 * fold would bury the accounts the caller is already showing; a term too short
 * to search says so instead of going quiet; and every row names whoever holds
 * the account, since binding one somebody else holds takes it off them.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AccountMatch } from "@/api/identity-client";

const hooks = vi.hoisted(() => ({
  search: {
    data: undefined as { pages: { items: AccountMatch[] }[] } | undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isPlaceholderData: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  },
}));
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  useAccountList: () => hooks.search,
}));

import { AccountPicker } from "./account-picker";

const PLACEHOLDER = "Find an account…";

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

function pick(over: { excludeKeys?: string[] } = {}) {
  const onPick = vi.fn();
  render(
    <AccountPicker
      onPick={onPick}
      placeholder={PLACEHOLDER}
      excludeKeys={over.excludeKeys}
    />,
  );
  return onPick;
}

const field = () => screen.getByRole("searchbox", { name: PLACEHOLDER });

beforeEach(() => {
  hooks.search.data = undefined;
  hooks.search.isFetching = false;
  hooks.search.isFetchingNextPage = false;
  hooks.search.isPlaceholderData = false;
  hooks.search.isError = false;
  hooks.search.hasNextPage = false;
  hooks.search.fetchNextPage.mockClear();
});

describe("AccountPicker", () => {
  it("lists nothing until the field is asked something", () => {
    hooks.search.data = { pages: [{ items: [match()] }] };
    pick();

    expect(screen.queryByText("annlee")).not.toBeInTheDocument();
  });

  // The answer to "why is nothing happening" should not wait for a debounce.
  it("says a single character is too short to search", async () => {
    pick();

    await userEvent.type(field(), "a");

    expect(screen.getByText(/at least 2 characters/i)).toBeInTheDocument();
  });

  it("hands the picked account back and writes nothing itself", async () => {
    hooks.search.data = { pages: [{ items: [match()] }] };
    const onPick = pick();

    await userEvent.type(field(), "annlee");
    await userEvent.click(screen.getByRole("button", { name: /^annlee$/ }));

    expect(onPick).toHaveBeenCalledWith(expect.objectContaining({ account_id: "zm-9" }));
  });

  // Binding an account somebody else holds takes it off them; that has to be
  // visible before the click, not in the confirmation after it.
  it("names the holder on the row, and states the two other answers", async () => {
    hooks.search.data = {
      pages: [
        {
          items: [
            match({ person: { person_id: "01900000-0000-7000-8000-0000000000b0", display_name: "Bob Park" } }),
            match({ account_id: "zm-10", username: "orphan" }),
            match({ account_id: "zm-11", username: "ci-bot", excluded: true }),
          ],
        },
      ],
    };
    pick();

    await userEvent.type(field(), "zm");

    expect(screen.getByText("Bob Park")).toBeInTheDocument();
    expect(screen.getByText(/bound to nobody/i)).toBeInTheDocument();
    expect(screen.getByText(/excluded — bot \/ CI/i)).toBeInTheDocument();
  });

  it("drops the accounts the caller excluded", async () => {
    hooks.search.data = {
      pages: [{ items: [match(), match({ account_id: "zm-10", username: "other" })] }],
    };
    pick({ excludeKeys: ["zoom:01900000-0000-7000-8000-00000000aa03:zm-9"] });

    await userEvent.type(field(), "zm");

    expect(screen.queryByText("annlee")).not.toBeInTheDocument();
    expect(screen.getByText("other")).toBeInTheDocument();
  });

  it("states a failed search rather than an empty list", async () => {
    hooks.search.isError = true;
    pick();

    await userEvent.type(field(), "annlee");

    expect(screen.getByText(/search failed/i)).toBeInTheDocument();
  });

  it("says nothing matched once the terms have been answered", async () => {
    hooks.search.data = { pages: [{ items: [] }] };
    pick();

    await userEvent.type(field(), "annlee");

    expect(screen.getByText(/no observed account carries that/i)).toBeInTheDocument();
  });

  // A page whose every row was excluded is not "nothing matches" while more
  // pages are unread — the next page is the answer, not the message.
  it("claims nothing while a further page is still unread", async () => {
    hooks.search.data = { pages: [{ items: [] }] };
    hooks.search.hasNextPage = true;
    pick();

    await userEvent.type(field(), "annlee");

    expect(
      screen.queryByText(/no observed account carries that/i),
    ).not.toBeInTheDocument();
  });
});
