/**
 * The account picker. What matters: it is a FINDING tool — the click hands the
 * account back and never writes; a blank field asks nothing, because the whole
 * fold would bury the accounts the caller is already showing; a term too short
 * to search says so instead of going quiet; and every row names whoever holds
 * the account, since binding one somebody else holds takes it off them.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AccountMatch } from "@/api/identity-client";
import { scrollEndIntoView } from "@/test/intersection-observer";

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

afterEach(() => vi.restoreAllMocks());

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
  it("lists nothing until the field is asked something", async () => {
    hooks.search.data = { pages: [{ items: [match()] }] };
    pick();

    expect(screen.queryByText("annlee")).not.toBeInTheDocument();
    // And the same rows DO arrive once asked — without this the case passes for
    // a picker that lists nothing ever.
    await userEvent.type(field(), "annlee");

    expect(screen.getByText("annlee")).toBeInTheDocument();
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
    await userEvent.click(screen.getByRole("button", { name: /^annlee,/ }));

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

  // The rows answer the term the query key carries, not the one in the field,
  // until the fetch lands. Marked rather than hidden, so the list does not
  // blank between keystrokes — and never read as the answer.
  it("marks the rows that still answer the previous term", async () => {
    hooks.search.data = { pages: [{ items: [match()] }] };
    hooks.search.isPlaceholderData = true;
    pick();

    await userEvent.type(field(), "annlee");

    expect(screen.getByRole("list")).toHaveAttribute("aria-busy", "true");
    expect(screen.getByText("annlee")).toBeInTheDocument();
  });

  // The marker asks for the next page, and it has to live INSIDE the scroller:
  // an observer never reports a target that is not a descendant of its root.
  it("asks for the next page when the end of the list comes into view", async () => {
    hooks.search.data = { pages: [{ items: [match()] }] };
    hooks.search.hasNextPage = true;
    pick();

    await userEvent.type(field(), "annlee");
    scrollEndIntoView();

    expect(hooks.search.fetchNextPage).toHaveBeenCalled();
  });

  // A page whose every row was filtered out has no rows to scroll, so the
  // marker is the only thing left that can ask for the page that does.
  it("keeps asking with every row filtered out", async () => {
    hooks.search.data = { pages: [{ items: [match()] }] };
    hooks.search.hasNextPage = true;
    pick({ excludeKeys: ["zoom:01900000-0000-7000-8000-00000000aa03:zm-9"] });

    await userEvent.type(field(), "annlee");
    scrollEndIntoView();

    expect(hooks.search.fetchNextPage).toHaveBeenCalled();
  });

  // Labelling an idle list "loading" is a lie the reader cannot dismiss.
  it("says it is loading only while the next page is on its way", async () => {
    hooks.search.data = { pages: [{ items: [match()] }] };
    hooks.search.hasNextPage = true;
    pick();
    await userEvent.type(field(), "annlee");

    expect(screen.queryByText(/loading/i)).not.toBeInTheDocument();
  });

  // Not a <button>: the holder's card carries its own copy control, and a
  // button may neither nest one nor let its text be selected.
  it("picks on Enter from the row, and not from a control inside it", async () => {
    hooks.search.data = {
      pages: [
        {
          items: [
            match({
              person: {
                person_id: "01900000-0000-7000-8000-0000000000b0",
                display_name: "Bob Park",
              },
            }),
          ],
        },
      ],
    };
    const onPick = pick();
    await userEvent.type(field(), "annlee");
    const row = screen.getByRole("button", { name: /^annlee,/ });

    await userEvent.click(within(row).getByRole("button", { name: /copy/i }));
    expect(onPick).not.toHaveBeenCalled();

    row.focus();
    await userEvent.keyboard("{Enter}");
    expect(onPick).toHaveBeenCalledTimes(1);
  });

  // Binding an account somebody else holds takes it off them, so the name the
  // row is reached by has to say so — not just the address.
  it("names the holder in the row's accessible name", async () => {
    hooks.search.data = {
      pages: [
        {
          items: [
            match({
              person: {
                person_id: "01900000-0000-7000-8000-0000000000b0",
                display_name: "Bob Park",
              },
            }),
          ],
        },
      ],
    };
    pick();

    await userEvent.type(field(), "annlee");

    expect(
      screen.getByRole("button", { name: /^annlee, zoom, held by Bob Park$/ }),
    ).toBeInTheDocument();
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
