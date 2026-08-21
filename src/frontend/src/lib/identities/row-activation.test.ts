/**
 * One rule for every console listing. What matters: a press on a control inside
 * a row is that control's — including a press that landed on the icon INSIDE
 * the control, which is what a real click reports; a click that ends a text
 * selection is a copy rather than a navigation; and only Enter and Space from
 * the row itself activate it.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { activatesRow, activatesRowByKey } from "./row-activation";

type MouseLike = React.MouseEvent<HTMLElement>;
type KeyLike = React.KeyboardEvent<HTMLElement>;

function click(target: EventTarget | null): MouseLike {
  return { target } as MouseLike;
}

function press(key: string, target: Element, currentTarget: Element): KeyLike {
  return { key, target, currentTarget } as unknown as KeyLike;
}

/** A row with one control in it, and whatever the control wraps. */
function row(control: "button" | "a", inner?: "svg" | "span") {
  const host = document.createElement("div");
  const el = document.createElement(control);
  host.append(el);
  if (!inner) return { host, target: el };
  const child = document.createElement(inner);
  el.append(child);
  return { host, target: child };
}

function selecting(collapsed: boolean) {
  vi.spyOn(window, "getSelection").mockReturnValue({
    isCollapsed: collapsed,
  } as Selection);
}

// The stub outlives the case that installed it otherwise, and the next case
// reads another one's selection state as its own.
afterEach(() => vi.restoreAllMocks());

describe("activatesRow", () => {
  it("activates on a click with nothing selected", () => {
    selecting(true);
    expect(activatesRow(click(document.createElement("span")))).toBe(true);
  });

  // A document with no selection at all is not a document with a selection
  // standing: the row still opens.
  it("activates where the platform reports no selection object", () => {
    vi.spyOn(window, "getSelection").mockReturnValue(null);
    expect(activatesRow(click(document.createElement("span")))).toBe(true);
  });

  it("leaves a press on a control inside the row to that control", () => {
    selecting(true);
    expect(activatesRow(click(row("button").target))).toBe(false);
  });

  // What a real click reports is the deepest element under the cursor — the
  // icon inside the copy button, never the button itself. A check on the
  // target's own tag would let every icon press open the row.
  it.each(["svg", "span"] as const)(
    "leaves a press on the %s inside a control to that control",
    (inner) => {
      selecting(true);
      expect(activatesRow(click(row("button", inner).target))).toBe(false);
    },
  );

  it("leaves a press on a link, and on what a link wraps, to the link", () => {
    selecting(true);
    expect(activatesRow(click(row("a").target))).toBe(false);
    expect(activatesRow(click(row("a", "span").target))).toBe(false);
  });

  // The addresses and ids on these rows are what an operator copies into a
  // ticket; opening on the mouse-up of a drag makes them unreadable.
  it("does not activate on the click that ends a selection", () => {
    selecting(false);
    expect(activatesRow(click(document.createElement("span")))).toBe(false);
  });
});

describe("activatesRowByKey", () => {
  it.each(["Enter", " "])("activates on %s from the row itself", (key) => {
    const host = document.createElement("div");
    expect(activatesRowByKey(press(key, host, host))).toBe(true);
  });

  it("ignores every other key", () => {
    const host = document.createElement("div");
    expect(activatesRowByKey(press("ArrowDown", host, host))).toBe(false);
  });

  // A control inside the row has its own answer to both keys.
  it("ignores the same keys pressed inside a control", () => {
    const { host, target } = row("button");
    expect(activatesRowByKey(press("Enter", target, host))).toBe(false);
  });
});
