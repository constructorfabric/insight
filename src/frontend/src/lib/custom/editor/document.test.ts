import { describe, expect, it } from "vitest";

import { spell } from "./describe";
import { change, hold, read, retype, sendable, written } from "./document";

describe("the document an editor holds", () => {
  it("shows a new definition as an empty object", () => {
    const held = hold();

    expect(held.document).toEqual({});
    expect(held.text).toBe("{}");
    expect(sendable(held)).toBe(true);
  });

  it("writes the text from the document when a field is edited", () => {
    const held = change(hold({ title: "Commits" }), ["title"], "Pull requests");

    expect(held.document).toEqual({ title: "Pull requests" });
    expect(JSON.parse(held.text)).toEqual({ title: "Pull requests" });
  });

  // The fields have nowhere to put a property they do not know, and dropping
  // it would turn a mistyped name into a silent loss rather than a refusal.
  it("keeps a property the fields do not know", () => {
    const held = change(
      hold({ title: "Commits", mistyped: 1 }),
      ["title"],
      "Renamed"
    );

    expect(held.document).toEqual({ title: "Renamed", mistyped: 1 });
  });

  it("removes a property rather than sending nothing under it", () => {
    const held = change(
      hold({ title: "Commits", description: "x" }),
      ["description"],
      undefined
    );

    expect(held.document).toEqual({ title: "Commits" });
    expect("description" in held.document).toBe(false);
  });

  it("makes the steps a path needs, so a field can be filled first", () => {
    const held = change(hold(), ["fields", 0, "name"], "author");

    expect(held.document).toEqual({ fields: [{ name: "author" }] });
  });

  it("takes an entry out of a list", () => {
    const held = change(
      hold({ fields: [{ name: "a" }, { name: "b" }] }),
      ["fields", 0],
      undefined
    );

    expect(held.document).toEqual({ fields: [{ name: "b" }] });
  });

  it("believes the text when it parses", () => {
    const held = retype(hold({ title: "Commits" }), '{"title":"Typed"}');

    expect(held.document).toEqual({ title: "Typed" });
    expect(held.unparsed).toBeUndefined();
    expect(sendable(held)).toBe(true);
  });

  // The fields must not show something nobody wrote, so they stay as they
  // stand and nothing may be sent until the text parses again.
  it("leaves the fields alone and blocks sending when the text does not parse", () => {
    const held = retype(hold({ title: "Commits" }), "{not json");

    expect(held.document).toEqual({ title: "Commits" });
    expect(held.text).toBe("{not json");
    expect(sendable(held)).toBe(false);
    expect(held.unparsed).toBeTruthy();
  });

  it("refuses text that parses to something a definition cannot be", () => {
    for (const text of ["[]", "42", '"a string"', "null"]) {
      const held = retype(hold({ title: "Commits" }), text);

      expect(sendable(held)).toBe(false);
      expect(held.document).toEqual({ title: "Commits" });
    }
  });

  it("sends again once the text parses", () => {
    const broken = retype(hold({ title: "Commits" }), "{");
    const mended = retype(broken, '{"title":"Mended"}');

    expect(sendable(mended)).toBe(true);
    expect(mended.document).toEqual({ title: "Mended" });
  });

  it("reads what sits at a path, and nothing where there is nothing", () => {
    const document = { fields: [{ name: "author" }] };

    expect(read(document, ["fields", 0, "name"])).toBe("author");
    expect(read(document, ["fields", 1, "name"])).toBeUndefined();
    expect(read(document, ["title"])).toBeUndefined();
  });

  it("writes a document a reader can edit by hand", () => {
    expect(written({ a: 1 })).toBe('{\n  "a": 1\n}');
  });
});

describe("the path a violation names", () => {
  it("is spelled the way the service spells it", () => {
    expect(spell(["title"])).toBe("title");
    expect(spell(["fields", 0, "type"])).toBe("fields[0].type");
    expect(spell(["row_identity", 1])).toBe("row_identity[1]");
    expect(spell([])).toBe("");
  });
});
