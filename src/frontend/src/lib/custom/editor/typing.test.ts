import { describe, expect, it } from "vitest";

import { indent, newline, outdent } from "./typing";

describe("indent", () => {
  it("puts an indent at the caret", () => {
    expect(indent("ab", 1, 1)).toEqual({ text: "a  b", start: 3, end: 3 });
  });

  it("replaces a selection within one line by an indent", () => {
    expect(indent("abcd", 1, 3)).toEqual({ text: "a  d", start: 3, end: 3 });
  });

  it("shifts every selected line and keeps them selected", () => {
    const text = "a\nb\nc";

    expect(indent(text, 0, 3)).toEqual({
      text: "  a\n  b\nc",
      start: 0,
      end: 7,
    });
  });
});

describe("outdent", () => {
  it("takes one indent off each selected line that has one", () => {
    expect(outdent("  a\nb\n    c", 0, 11)).toEqual({
      text: "a\nb\n  c",
      start: 0,
      end: 7,
    });
  });

  it("moves the caret back by what it took from its line", () => {
    expect(outdent("  a", 3, 3)).toEqual({ text: "a", start: 1, end: 1 });
  });

  it("leaves a line with no indent alone", () => {
    expect(outdent("a", 1, 1)).toEqual({ text: "a", start: 1, end: 1 });
  });
});

describe("newline", () => {
  it("indents the new line as far as the one it leaves", () => {
    expect(newline('  "a": 1,', 9, 9)).toEqual({
      text: '  "a": 1,\n  ',
      start: 12,
      end: 12,
    });
  });

  it("replaces a selection", () => {
    expect(newline("ab", 0, 2)).toEqual({ text: "\n", start: 1, end: 1 });
  });
});
