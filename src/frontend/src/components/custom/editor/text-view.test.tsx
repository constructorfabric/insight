import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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

  // JSON is indented by hand here, and a Tab that left the field would make
  // that a chore.
  it("indents on Tab instead of leaving the field", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<TextView id="t" label="as text" text="ab" onChange={onChange} />);

    const field = screen.getByLabelText("as text") as HTMLTextAreaElement;
    field.focus();
    field.setSelectionRange(1, 1);
    await user.keyboard("{Tab}");

    expect(onChange).toHaveBeenCalledWith("a  b");
    expect(field).toHaveFocus();
  });

  it("keeps the indent of the line Enter leaves", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<TextView id="t" label="as text" text="  a" onChange={onChange} />);

    const field = screen.getByLabelText("as text") as HTMLTextAreaElement;
    field.focus();
    field.setSelectionRange(3, 3);
    await user.keyboard("{Enter}");

    expect(onChange).toHaveBeenCalledWith("  a\n  ");
  });

  it("lets a keyboard out on Escape, since Tab is taken", async () => {
    const user = userEvent.setup();
    render(<TextView id="t" label="as text" text="a" onChange={vi.fn()} />);

    const field = screen.getByLabelText("as text");
    field.focus();
    await user.keyboard("{Escape}");

    expect(field).not.toHaveFocus();
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
