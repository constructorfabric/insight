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
    isPlaceholderData: false,
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

import { SCROLL_ENDS_AFTER_ROWS } from "@/components/widgets/scroll-to-ends";
import { MAX_SEARCH_CHARS } from "@/queries/identity-resolution";
import { scrollEndIntoView } from "@/test/intersection-observer";

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
    isPlaceholderData: false,
    isError: false,
    hasNextPage: false,
    fetchNextPage: vi.fn(),
  };
});

describe("PersonPicker", () => {
  // Choosing a person unmounts this field, so what was typed has to live
  // somewhere that outlives it — otherwise coming back means finding the same
  // person twice.
  it("starts from the terms it was handed", () => {
    render(<PersonPicker onPick={vi.fn()} initialQuery="park" />);

    expect(screen.getByRole("searchbox")).toHaveValue("park");
  });

  // Debounced, not per keystroke: a caller that stores these in the URL would
  // otherwise navigate on every letter.
  it("reports the terms once the reader has stopped typing", async () => {
    const onSettled = vi.fn();
    render(<PersonPicker onPick={vi.fn()} onSettled={onSettled} />);

    await userEvent.type(screen.getByRole("searchbox"), "park");

    expect(onSettled).toHaveBeenLastCalledWith("park");
  });

  // One letter names most of the roster: the answer is no use and the service
  // pays a pass over the journal for it. Going silent instead would read as a
  // broken field, so the picker says what it is waiting for.
  // `/v1/persons` refuses a needle past 200 characters, the same ceiling the
  // account search carries. The field stops there rather than letting a paste
  // become a refusal the reader cannot interpret.
  it("stops at the length the service accepts", () => {
    render(<PersonPicker onPick={vi.fn()} />);

    expect(screen.getByRole("searchbox")).toHaveAttribute(
      "maxlength",
      String(MAX_SEARCH_CHARS),
    );
  });

  it("asks for a second character instead of searching on one", async () => {
    list.state.data = { pages: [page([BOB])] };
    render(<PersonPicker onPick={vi.fn()} />);

    await userEvent.type(screen.getByRole("searchbox"), "b");

    expect(screen.getByText(/at least 2 characters/i)).toBeInTheDocument();
    expect(screen.queryByText("Bob Park")).not.toBeInTheDocument();
    expect(screen.queryByText(/nobody matches/i)).not.toBeInTheDocument();
  });

  it("searches, and drops the notice, on the second character", async () => {
    list.state.data = { pages: [page([BOB])] };
    render(<PersonPicker onPick={vi.fn()} />);

    await userEvent.type(screen.getByRole("searchbox"), "bo");

    expect(screen.queryByText(/at least 2 characters/i)).not.toBeInTheDocument();
    expect(screen.getByText("Bob Park")).toBeInTheDocument();
  });

  // The service matches every term against the journal on its own, so a term
  // under the floor is a pass over the journal no matter what precedes it.
  it("holds back while any one word is still a single character", async () => {
    list.state.data = { pages: [page([BOB])] };
    render(<PersonPicker onPick={vi.fn()} />);

    await userEvent.type(screen.getByRole("searchbox"), "bob p");

    expect(screen.getByText(/at least 2 characters/i)).toBeInTheDocument();
    expect(screen.queryByText("Bob Park")).not.toBeInTheDocument();
  });

  // The roster is not a search, so the console's own mode still opens with it.
  it("keeps the roster on an empty field where the caller asked for one", () => {
    list.state.data = { pages: [page([BOB])] };
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    expect(screen.getByText("Bob Park")).toBeInTheDocument();
    expect(screen.queryByText(/at least 2 characters/i)).not.toBeInTheDocument();
  });

  it("hands the picked person to the caller and fires nothing else", async () => {
    list.state.data = { pages: [page([BOB])] };
    const onPick = vi.fn();
    render(<PersonPicker onPick={onPick} />);

    await userEvent.type(screen.getByRole("searchbox"), "bob");
    await userEvent.click(screen.getByRole("button", { name: /bob park/i }));

    expect(onPick).toHaveBeenCalledWith(BOB);
  });

  // The rows survive a keystroke so the list does not blank mid-word, which
  // means what is on screen can be the PREVIOUS term's answer. Say so, or the
  // operator reads a list one term behind as the answer to what they typed.
  it("marks the list as busy while it answers the previous term", () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.isPlaceholderData = true;
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    expect(screen.getByRole("list")).toHaveAttribute("aria-busy", "true");
  });

  // Marked, not disabled: the row carries the name the operator read off it, so
  // the click is never for the wrong person — and picking only opens the step
  // that fires a verb. Swallowing it would make the picker feel broken.
  it("still picks a row while the list is one term behind", async () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.isPlaceholderData = true;
    const onPick = vi.fn();
    render(<PersonPicker onPick={onPick} browseWhenEmpty />);

    await userEvent.click(screen.getByRole("button", { name: /bob park/i }));

    expect(onPick).toHaveBeenCalledWith(BOB);
  });

  it("drops the busy mark once the rows answer the field", () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.isPlaceholderData = false;
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    expect(screen.getByRole("list")).toHaveAttribute("aria-busy", "false");
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

  // A roster is read by scrolling it, so the next page arrives on the way down
  // rather than behind a button the reader has to notice and press.
  it("asks for the next page when the end of the list comes into view", () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.hasNextPage = true;
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    scrollEndIntoView();

    expect(list.state.fetchNextPage).toHaveBeenCalled();
  });

  it("asks once, not once per frame, while a page is already in flight", () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.hasNextPage = true;
    list.state.isFetchingNextPage = true;
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    scrollEndIntoView();
    scrollEndIntoView();

    expect(list.state.fetchNextPage).not.toHaveBeenCalled();
  });

  // The marker is what asks for the next page, and a page whose every row was
  // excluded has no rows for it to sit under — so it has to be rendered anyway,
  // or the reader is stuck on a list that is empty and has more to give.
  it("still reaches the next page when every row of this one was excluded", () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.hasNextPage = true;
    render(<PersonPicker onPick={vi.fn()} excludeIds={[BOB.person_id]} browseWhenEmpty />);

    scrollEndIntoView();

    expect(list.state.fetchNextPage).toHaveBeenCalled();
  });

  // Scrolling back by hand up a roster that keeps growing is the cost this
  // exists to remove, so it appears only once the list is actually long.
  it("offers the jump-to-either-end control only on a long list", () => {
    const many = Array.from({ length: SCROLL_ENDS_AFTER_ROWS + 1 }, (_, i) => ({
      person_id: `01900000-0000-7000-8000-${String(i).padStart(12, "0")}`,
      display_name: `Person ${i}`,
    }));
    list.state.data = { pages: [page([BOB])] };
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty asSurface />);
    expect(
      screen.queryByRole("button", { name: /back to the top/i }),
    ).not.toBeInTheDocument();

    list.state.data = { pages: [page(many)] };
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty asSurface />);

    expect(
      screen.getAllByRole("button", { name: /back to the top/i }),
    ).not.toHaveLength(0);
    expect(
      screen.getAllByRole("button", { name: /jump to the end/i }),
    ).not.toHaveLength(0);
  });

  // Inside a panel it is a field with a short list under it; a full-height
  // scroller with its own card would take the dialog over.
  it("stays a plain short list inside a panel", () => {
    list.state.data = { pages: [page([BOB])] };
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    expect(
      screen.queryByRole("button", { name: /back to the top/i }),
    ).not.toBeInTheDocument();
  });

  it("stops asking once the listing says there is no next page", () => {
    list.state.data = { pages: [page([BOB])] };
    list.state.hasNextPage = false;
    render(<PersonPicker onPick={vi.fn()} browseWhenEmpty />);

    scrollEndIntoView();

    expect(list.state.fetchNextPage).not.toHaveBeenCalled();
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
    // The unread pages are the answer, and they arrive by scrolling.
    scrollEndIntoView();
    expect(list.state.fetchNextPage).toHaveBeenCalled();
  });
});
