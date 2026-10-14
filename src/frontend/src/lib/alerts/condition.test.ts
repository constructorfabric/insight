import { describe, expect, it } from "vitest";

import {
  blankLeaf,
  canNegate,
  fitCondition,
  groupDepth,
  leafPaths,
  nodeAt,
  placeRefusals,
  withListValues,
  type ColumnKind,
} from "./condition";

const OPEN = { column: "open", op: ">", value: 10 };
const TEAM = { column: "team", op: "==", value: "core" };
const STALE = { column: "stale", op: ">", value: 3 };

/** open and (team or not stale) */
const NESTED = { all: [OPEN, { any: [TEAM, { not: STALE }] }] };

describe("leafPaths", () => {
  it.each([
    ["a single test", OPEN, ["where"]],
    [
      "a nested tree",
      NESTED,
      ["where.all[0]", "where.all[1].any[0]", "where.all[1].any[1].not"],
    ],
    ["something that is not a condition", ["open"], []],
    ["an empty group", { all: [] }, []],
  ])("lists the tests of %s in reading order", (_case, where, paths) => {
    expect(leafPaths(where), JSON.stringify(where)).toEqual(paths);
  });
});

describe("nodeAt", () => {
  it.each([
    ["where", NESTED],
    ["where.all[0]", OPEN],
    ["where.all[1].any[0]", TEAM],
    ["where.all[1].any[1]", { not: STALE }],
    ["where.all[1].any[1].not", STALE],
    ["where.all[5]", undefined],
    ["where.any[0]", undefined],
  ])("finds the node at %s", (path, node) => {
    expect(nodeAt(NESTED, path), path).toEqual(node);
  });
});

describe("groupDepth", () => {
  it.each([
    ["where", 1],
    ["where.all[0]", 1],
    ["where.all[1]", 2],
    ["where.all[1].any[0]", 2],
    ["where.all[1].any[1]", 3],
    ["where.all[1].any[1].not", 3],
  ])("counts the groups at and above %s", (path, depth) => {
    expect(groupDepth(NESTED, path), path).toBe(depth);
  });

  it("counts no group for a single test", () => {
    expect(groupDepth(OPEN, "where")).toBe(0);
  });
});

describe("canNegate", () => {
  it.each([
    ["a group of two tests", { all: [OPEN, TEAM] }, "where", true],
    [
      "a group of one subgroup",
      { all: [{ any: [{ all: [OPEN] }] }] },
      "where",
      true,
    ],
    [
      "a root group whose subgroups already reach the bound",
      { all: [OPEN, { any: [TEAM, { all: [STALE] }] }] },
      "where",
      false,
    ],
    [
      "a nested group of two tests at the second level",
      { all: [OPEN, { any: [TEAM, STALE] }] },
      "where.all[1]",
      true,
    ],
    [
      "a nested group of two tests with a subgroup",
      { all: [OPEN, { any: [TEAM, { all: [STALE] }] }] },
      "where.all[1]",
      false,
    ],
    [
      "a group that is already a not",
      { not: { all: [OPEN, TEAM] } },
      "where",
      true,
    ],
  ])("says whether %s can become a not", (_case, where, path, allowed) => {
    expect(canNegate(where, path), JSON.stringify(where)).toBe(allowed);
  });
});

describe("fitCondition", () => {
  const kinds = new Map<string, ColumnKind>([
    ["open", "number"],
    ["team", "text"],
  ]);

  it.each([
    ["a test whose column and operator still fit", OPEN, OPEN],
    ["a test whose column is gone", STALE, blankLeaf()],
    [
      "a test whose column now holds another kind",
      { column: "team", op: ">", value: 1 },
      blankLeaf(),
    ],
    [
      "a nested tree",
      NESTED,
      { all: [OPEN, { any: [TEAM, { not: blankLeaf() }] }] },
    ],
  ])("keeps %s only where it fits the new metric", (_case, where, fitted) => {
    expect(
      fitCondition({ rows: "any", where }, kinds),
      JSON.stringify(where)
    ).toEqual({ rows: "any", where: fitted });
  });
});

describe("placeRefusals", () => {
  it("puts each refusal on the deepest node its path names", () => {
    const placed = placeRefusals(
      {
        name: "Enter a name.",
        condition: "must be an object",
        "condition.rows.count.value": "must be a whole number",
        "condition.where.all": "a group needs at least one condition",
        "condition.where.all[1].any[0].op": "wrong operator",
        "condition.where.all[1].any[1].not.value": "not a number",
        "condition.where.all[10].value": "past the end",
      },
      NESTED
    );

    expect(Object.fromEntries(placed.atNode)).toEqual({
      where: {
        "where.all": "a group needs at least one condition",
        "where.all[10].value": "past the end",
      },
      "where.all[1].any[0]": { "where.all[1].any[0].op": "wrong operator" },
      "where.all[1].any[1].not": {
        "where.all[1].any[1].not.value": "not a number",
      },
    });
    expect(placed.rows).toEqual({
      "rows.count.value": "must be a whole number",
    });
    expect(placed.whole).toEqual(["must be an object"]);
  });
});

describe("withListValues", () => {
  const full = Array.from({ length: 50 }, (_, at) => `v${at}`);

  it.each([
    [
      "trims and adds",
      [],
      ["  core ", "infra"],
      ["core", "infra"],
      undefined,
      [],
    ],
    [
      "skips empty and repeated",
      ["core"],
      ["", " ", "core", "core "],
      ["core"],
      undefined,
      [],
    ],
    [
      "refuses past 50 values",
      full,
      ["extra", "more"],
      full,
      "At most 50 values.",
      ["extra", "more"],
    ],
    [
      "refuses past 256 characters and keeps what came before",
      [],
      ["core", "x".repeat(257), "infra"],
      ["core"],
      "At most 256 characters.",
      ["x".repeat(257), "infra"],
    ],
    [
      "accepts 256 characters",
      [],
      ["x".repeat(256)],
      ["x".repeat(256)],
      undefined,
      [],
    ],
  ])("%s", (_case, current, entries, values, refused, unadded) => {
    expect(withListValues(current, entries), _case).toEqual({
      values,
      unadded,
      ...(refused ? { refused } : {}),
    });
  });
});
