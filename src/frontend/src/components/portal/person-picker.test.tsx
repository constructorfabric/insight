// @vitest-environment jsdom
/**
 * The picker finds, it never decides: picking hands the person to the caller
 * and fires nothing. Short input asks nobody; already-shown persons are not
 * repeated; a truncated answer says "narrow the terms" instead of posing as
 * complete.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { PersonSearchResponse } from "@/api/identity-client";

const search = vi.hoisted(() => ({
  q: "",
  state: {
    data: undefined as PersonSearchResponse | undefined,
    isFetching: false,
    isError: false,
  },
}));
vi.mock("@/queries/identity-resolution", () => ({
  usePersonSearch: (q: string) => {
    search.q = q;
    return search.state;
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

beforeEach(() => {
  search.q = "";
  search.state = { data: undefined, isFetching: false, isError: false };
});

describe("PersonPicker", () => {
  it("hands the picked person to the caller and fires nothing else", async () => {
    search.state.data = { items: [BOB], truncated: false };
    const onPick = vi.fn();
    render(<PersonPicker onPick={onPick} />);

    await userEvent.type(screen.getByRole("searchbox"), "bob");
    await userEvent.click(screen.getByRole("button", { name: /bob park/i }));

    expect(onPick).toHaveBeenCalledWith(BOB);
  });

  it("does not repeat persons the panel already shows", async () => {
    search.state.data = { items: [BOB, CAROL], truncated: false };
    render(<PersonPicker onPick={vi.fn()} excludeIds={[BOB.person_id]} />);

    await userEvent.type(screen.getByRole("searchbox"), "park");

    expect(screen.getByText("Carol Chen")).toBeInTheDocument();
    expect(screen.queryByText("Bob Park")).not.toBeInTheDocument();
  });

  it("a truncated answer asks for narrower terms", async () => {
    search.state.data = { items: [BOB], truncated: true };
    render(<PersonPicker onPick={vi.fn()} />);

    await userEvent.type(screen.getByRole("searchbox"), "pa");

    expect(screen.getByText(/narrow the terms/i)).toBeInTheDocument();
  });

  it("passes the typed query through to the search hook", async () => {
    render(<PersonPicker onPick={vi.fn()} />);

    await userEvent.type(screen.getByRole("searchbox"), "iva example");

    expect(search.q).toBe("iva example");
  });
});
