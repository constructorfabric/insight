import { describe, expect, it } from "vitest";

import { addressOf, dotted, qualifies, spelled } from "./source";

describe("addressOf", () => {
  // The service returns early on a `database` of its own and never splits the
  // name beside one. Splitting here would offer another table's columns.
  it("lets a database of its own win over the one a name carries", () => {
    expect(addressOf("silver.fct_commit", "insight")).toEqual({
      database: "insight",
      table: "silver.fct_commit",
    });
  });

  it("splits a qualified name written on its own", () => {
    expect(addressOf("silver.fct_commit", undefined)).toEqual({
      database: "silver",
      table: "fct_commit",
    });
    expect(addressOf("silver.fct_commit", "")).toEqual({
      database: "silver",
      table: "fct_commit",
    });
  });

  it("leaves a name the service would not split whole", () => {
    for (const written of ["a.b.c", ".table", "table.", "a b.c", "fct_commit"]) {
      expect(addressOf(written, ""), written).toEqual({
        database: "",
        table: written,
      });
    }
  });

  it("has no address for nothing written", () => {
    expect(addressOf("", "silver")).toBeUndefined();
    expect(addressOf(undefined, "silver")).toBeUndefined();
    expect(addressOf(7, "silver")).toBeUndefined();
  });
});

describe("qualifies", () => {
  it.each([
    ["silver.fct_commit", true],
    ["fct_commit", false],
    ["a.b.c", false],
    [".fct", false],
    ["silver.", false],
    ["sil ver.fct", false],
    [7, false],
  ])("reads %j as %s", (written, expected) => {
    expect(qualifies(written)).toBe(expected);
  });
});

describe("spelled", () => {
  it("writes a database back onto the name, and leaves a bare one bare", () => {
    expect(spelled({ database: "silver", table: "fct" })).toBe("silver.fct");
    expect(spelled({ database: "", table: "fct" })).toBe("fct");
  });
});

describe("dotted", () => {
  // The service takes either form and compiles both the same; the editor asks
  // for one name, so what is stored settles on the one form.
  it("joins a database of its own onto the name", () => {
    expect(dotted({ database: "silver", table: "fct", fields: [] })).toEqual({
      table: "silver.fct",
      fields: [],
    });
  });

  it("leaves a name that already carries its database", () => {
    const body = { table: "silver.fct" };

    expect(dotted(body)).toEqual(body);
  });

  it("leaves a body with nothing to join", () => {
    for (const body of [
      { table: "fct" },
      { database: "", table: "fct" },
      { database: "silver" },
      { database: "silver", table: "" },
      { dataset: "commits" },
    ]) {
      expect(dotted(body), JSON.stringify(body)).toEqual(body);
    }
  });

  // Joining what the service would refuse keeps the refusal, rather than
  // inventing a name that reads differently.
  it("joins a name the service would refuse just the same", () => {
    expect(dotted({ database: "a", table: "b.c" })).toEqual({
      table: "a.b.c",
    });
  });
});
