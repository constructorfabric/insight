import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { brokenLine } from "@/lib/custom/editor/document";

import { TextView } from "./text-view";

describe("<TextView>", () => {
  it("numbers every line of the text", () => {
    render(
      <TextView
        id="t"
        label="as text"
        text={'{\n  "a": 1,\n  "b": 2\n}'}
        onChange={vi.fn()}
      />
    );

    const gutter = screen.getByLabelText("as text").previousElementSibling;
    expect(gutter?.textContent).toBe("1234");
  });

  it("marks the line the parser could not read", () => {
    render(
      <TextView
        id="t"
        label="as text"
        text={"{\n  x\n}"}
        unparsed="Unexpected token x in JSON at position 4 (line 2 column 3)"
        onChange={vi.fn()}
      />
    );

    const gutter = screen.getByLabelText("as text").previousElementSibling;
    const second = gutter?.children[1];
    expect(second?.className).toContain("text-destructive");
    expect(gutter?.children[0]?.className).not.toContain("text-destructive");
    expect(screen.getByRole("alert")).toHaveTextContent("line 2");
  });

  it("does not wrap lines, so a number always faces its line", () => {
    render(<TextView id="t" label="as text" text="{}" onChange={vi.fn()} />);

    expect(screen.getByLabelText("as text")).toHaveAttribute("wrap", "off");
  });
});

describe("brokenLine", () => {
  it.each([
    [
      "Bad control character in string literal in JSON at position 1670 (line 18 column 152)",
      18,
    ],
    ["Unexpected end of JSON input", undefined],
    [undefined, undefined],
  ])("reads the line out of %j", (message, expected) => {
    expect(brokenLine(message)).toBe(expected);
  });
});
