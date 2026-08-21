/**
 * One rule for every console listing. What matters: a press on a control inside
 * a row is that control's, a click that ends a text selection is a copy rather
 * than a navigation, and only Enter and Space from the row itself activate it.
 */
import { describe, expect, it, vi } from "vitest";

import { activatesRow, activatesRowByKey } from "./row-activation";

type MouseLike = React.MouseEvent<HTMLElement>;
type KeyLike = React.KeyboardEvent<HTMLElement>;

function click(target: EventTarget | null): MouseLike {
  return { target } as MouseLike;
}

function press(key: string, target: Element, currentTarget: Element): KeyLike {
  return { key, target, currentTarget } as unknown as KeyLike;
}

function selecting(collapsed: boolean) {
  vi.spyOn(window, "getSelection").mockReturnValue({
    isCollapsed: collapsed,
  } as Selection);
}

describe("activatesRow", () => {
  it("activates on a click with nothing selected", () => {
    selecting(true);
    expect(activatesRow(click(document.createElement("span")))).toBe(true);
  });

  it("leaves a press on a control inside the row to that control", () => {
    selecting(true);
    const row = document.createElement("div");
    const button = document.createElement("button");
    row.append(button);
    expect(activatesRow(click(button))).toBe(false);
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
    const row = document.createElement("div");
    expect(activatesRowByKey(press(key, row, row))).toBe(true);
  });

  it("ignores every other key", () => {
    const row = document.createElement("div");
    expect(activatesRowByKey(press("ArrowDown", row, row))).toBe(false);
  });

  // A control inside the row has its own answer to both keys.
  it("ignores the same keys pressed inside a control", () => {
    const row = document.createElement("div");
    const button = document.createElement("button");
    expect(activatesRowByKey(press("Enter", button, row))).toBe(false);
  });
});
