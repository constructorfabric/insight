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
  enabled: true as boolean,
  state: {
    data: undefined as { pages: PersonSearchResponse[] } | undefined,
    isFetching: false,
    isFetchingNextPage: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  },
}));
vi.mock("@/queries/identity-resolution", () => ({
  usePersonList: (q: string, options?: { enabled?: boolean }) => {
    list.q = q;
    list.enabled = options?.enabled ?? true;
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
  return { items, truncated: false };
}

beforeEach(() => {
  list.q = "";
  list.enabled = true;
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
    render(<PersonPicker onPick={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: /show more/i }));

    expect(list.state.fetchNextPage).toHaveBeenCalled();
  });

  it("an empty field lists the roster only where the caller asked for it", () => {
    // Inside the assign dialog an empty field must stay silent: listing the
    // tenant there would bury the one name the operator came to type.
    const { unmount } = render(<PersonPicker onPick={vi.fn()} />);
    expect(list.enabled).toBe(false);
    unmount();

    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);
    expect(list.enabled).toBe(true);
  });

  it("passes the typed query through to the listing hook", async () => {
    render(<PersonPicker onPick={vi.fn()} />);

    await userEvent.type(screen.getByRole("searchbox"), "iva example");

    expect(list.q).toBe("iva example");
  });
});
