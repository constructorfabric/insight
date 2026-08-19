// @vitest-environment jsdom
/**
 * The picker finds, it never decides: picking hands the person to the caller
 * and fires nothing. Already-shown persons are not repeated; a list longer
 * than a page keeps going instead of asking for narrower terms; and an empty
 * field means the roster only where the caller said it should.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { PersonSearchResponse } from "@/api/identity-client";

const list = vi.hoisted(() => ({
  q: "",
  intent: "browse" as "browse" | "match",
  state: {
    data: undefined as { pages: PersonSearchResponse[] } | undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  },
}));
// Only the hook is stubbed: `listsAnyone` stays the real rule, since what the
// picker displays and what the query asks for must not drift apart.
vi.mock("@/queries/identity-resolution", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-resolution")>()),
  usePersonList: (q: string, intent: "browse" | "match" = "browse") => {
    list.q = q;
    list.intent = intent;
    return list.state;
  },
}));
// The debounce is a pure timing concern with its own unit test; identity here
// keeps the picker test about behaviour, not timers.
vi.mock("@/hooks/use-debounced-value", () => ({
  useDebouncedValue: <T,>(value: T) => value,
}));

import { PersonPicker } from "./person-picker";

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

function page(items: PersonSearchResponse["items"]): PersonSearchResponse {
  return { items };
}

beforeEach(() => {
  list.q = "";
  list.intent = "browse";
  list.state = {
    data: undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  };
});

describe("PersonPicker", () => {
  it("hands the picked person to the caller and fires nothing else", async () => {
    list.state.data = { pages: [page([BOB])] };
    const onPick = vi.fn();
    render(<PersonPicker onPick={onPick} />);

    await userEvent.type(screen.getByRole("searchbox"), "bob");
    await userEvent.click(screen.getByRole("button", { name: /bob park/i }));

    expect(onPick).toHaveBeenCalledWith(BOB);
  });

  it("does not repeat persons the panel already shows", async () => {
    list.state.data = { pages: [page([BOB, CAROL])] };
    render(<PersonPicker onPick={vi.fn()} excludeIds={[BOB.person_id]} />);

    await userEvent.type(screen.getByRole("searchbox"), "park");

    expect(screen.getByText("Carol Chen")).toBeInTheDocument();
    expect(screen.queryByText("Bob Park")).not.toBeInTheDocument();
  });

  it("shows every page fetched so far as one list", () => {
    list.state.data = { pages: [page([BOB]), page([CAROL])] };
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    expect(screen.getByText("Bob Park")).toBeInTheDocument();
    expect(screen.getByText("Carol Chen")).toBeInTheDocument();
  });

  it("asks for the next page instead of telling the operator to narrow the terms", async () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.hasNextPage = true;
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    await userEvent.click(screen.getByRole("button", { name: /show more/i }));

    expect(list.state.fetchNextPage).toHaveBeenCalled();
  });

  it.each([
    ["the dialog's field", false, "match" as const],
    ["the console's own mode", true, "browse" as const],
  ])(
    "tells the listing what an empty field means in %s",
    (_case, browseWhenEmpty, expected) => {
      render(
        <PersonPicker onPick={vi.fn()} browseWhenEmpty={browseWhenEmpty} />,
      );

      expect(list.intent).toBe(expected);
    },
  );

  // The tenant may sit in the cache under the browse key. Whatever the hook
  // hands back, a field that asked for nothing shows nothing — otherwise the
  // assign dialog lists the roster the person mode just browsed.
  it("shows nothing where an empty field means matches, even with rows in hand", () => {
    list.state.data = { pages: [page([BOB, CAROL])] };
    list.state.hasNextPage = true;

    render(<PersonPicker onPick={vi.fn()} />);

    expect(screen.queryByText("Bob Park")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /show more/i }),
    ).not.toBeInTheDocument();
  });

  it("still searches once terms are typed into that same field", async () => {
    list.state.data = { pages: [page([BOB])] };
    render(<PersonPicker onPick={vi.fn()} />);

    await userEvent.type(screen.getByRole("searchbox"), "iva example");

    expect(list.q).toBe("iva example");
    expect(list.intent).toBe("match");
    expect(screen.getByText("Bob Park")).toBeInTheDocument();
  });

  // Excluding is client-side, so a page can arrive whole and leave empty while
  // later pages hold matches. "Nobody matches" beside a live button is a
  // contradiction the operator can only resolve by guessing.
  it("offers the next page instead of claiming nobody matches", async () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.hasNextPage = true;
    render(<PersonPicker onPick={vi.fn()} excludeIds={[BOB.person_id]} />);

    await userEvent.type(screen.getByRole("searchbox"), "park");

    expect(screen.queryByText(/nobody matches/i)).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /show more/i }),
    ).toBeInTheDocument();
  });
});
