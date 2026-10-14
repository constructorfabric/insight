import { describe, expect, it } from "vitest";

import {
  cellText,
  conditionText,
  matchedText,
  reasonText,
  statusText,
} from "./describe";

function one(where: unknown) {
  return { rows: "one", where };
}

describe("conditionText", () => {
  it.each([
    [one({ column: "open", op: ">", value: 10 }), "open above 10"],
    [
      one({ column: "open", op: "between", value: [0.25, 1] }),
      "open between 0.25 and 1",
    ],
    [
      {
        rows: "any",
        where: {
          all: [
            { column: "open", op: ">", value: 10 },
            {
              any: [
                { column: "team", op: "==", value: "core" },
                { column: "stale", op: ">", value: 3 },
              ],
            },
          ],
        },
      },
      "any row: open above 10 and (team is core or stale above 3)",
    ],
    [
      {
        rows: "none",
        where: { not: { column: "team", op: "in", value: ["core", "infra"] } },
      },
      "no row: not (team is one of core, infra)",
    ],
    [
      {
        rows: { count: { op: ">", value: 3 } },
        where: { column: "opened_at", op: "older_than", value: "P1D" },
      },
      "number of matching rows above 3: opened_at older than 1 day",
    ],
    [
      {
        rows: "all",
        where: {
          column: "opened_at",
          op: "before",
          value: "2026-10-01T09:05:00Z",
        },
      },
      "every row: opened_at before 2026-10-01 09:05 UTC",
    ],
    [
      one({
        column: "opened_at",
        op: "between",
        value: ["2026-09-01T00:00:00Z", "2026-10-01T12:30:00.500Z"],
      }),
      "opened_at between 2026-09-01 00:00 UTC and 2026-10-01 12:30 UTC",
    ],
    [
      one({ column: "team", op: "starts_with", value: "co" }),
      "team starts with co",
    ],
    [
      one({ column: "team", op: "not_in", value: ["core"] }),
      "team is none of core",
    ],
    [
      one({ column: "opened_at", op: "newer_than", value: "PT30M" }),
      "opened_at newer than 30 minutes",
    ],
    [
      one({ column: "open", op: "<=", value: "9007199254740993" }),
      "open at or below 9007199254740993",
    ],
  ])("words %j as the service does", (condition, words) => {
    expect(conditionText(condition), JSON.stringify(condition)).toBe(words);
  });
});

describe("a threshold in a condition's words", () => {
  it.each([
    [10, "10"],
    [0.25, "0.25"],
    [123456.5, "123456.5"],
    [1234567.25, "1234567.25"],
    [1e-20, "1e-20"],
    [1e21, "1e+21"],
    [-2.5, "-2.5"],
    ["98765432109876543210", "98765432109876543210"],
    ["007", "007"],
  ])("writes %s exactly as %s", (value, text) => {
    expect(
      conditionText(one({ column: "open", op: ">", value })),
      `value: ${value}`
    ).toBe(`open above ${text}`);
  });

  it.each([
    [[123456.5, 1e-20], "open between 123456.5 and 1e-20"],
    [["9007199254740993", 2.5], "open between 9007199254740993 and 2.5"],
  ])("writes between bounds %j exactly", (value, words) => {
    expect(
      conditionText(one({ column: "open", op: "between", value })),
      JSON.stringify(value)
    ).toBe(words);
  });
});

describe("describe", () => {
  it.each([
    [null, "—"],
    [1200, "1,200"],
    [1234567.25, "1,234,567.25"],
    ["98765432109876543210", "98765432109876543210"],
    ["core", "core"],
    [true, "true"],
  ])("shows the cell %j as %s", (cell, text) => {
    expect(cellText(cell), `cell: ${JSON.stringify(cell)}`).toBe(text);
  });

  it.each([
    [3, 12, "3 of 12 rows match"],
    [1, 1, "1 of 1 row matches"],
    [0, 0, "0 of 0 rows match"],
  ])("counts %s of %s", (matched, answered, text) => {
    expect(matchedText({ matched, answered, columns: [], rows: [] })).toBe(
      text
    );
  });

  it("says why for every reason code the service sends", () => {
    for (const code of [
      "no_rows",
      "many_rows",
      "column_missing",
      "null",
      "non_numeric",
      "non_text",
      "non_date",
      "incomparable",
      "row_limit",
      "metric_missing",
      "compile_failed",
      "run_failed",
      "timeout",
    ]) {
      expect(reasonText(code), `should explain: ${code}`).not.toBe(code);
    }
  });

  it("shows a reason or status it does not know as it came", () => {
    expect(reasonText("brand_new")).toBe("brand_new");
    expect(statusText("queued")).toBe("queued");
  });

  it.each([
    ["pending", "Pending"],
    ["cancelled", "Cancelled"],
    ["sent", "Sent"],
    ["failed", "Failed"],
  ])("names status %s as %s", (status, text) => {
    expect(statusText(status)).toBe(text);
  });
});
