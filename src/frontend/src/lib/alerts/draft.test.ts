import { describe, expect, it } from "vitest";

import type { Alert } from "@/api/alerts-types";

import type { ColumnKind } from "./condition";
import { hold } from "@/lib/custom/editor/document";

import {
  blankForm,
  checkForm,
  formOf,
  numberOf,
  parseDraftText,
  retypeDraft,
} from "./draft";

/** The longest name the service accepts, as the form's message states it. */
const NAME_MAX = 200;

const NESTED = {
  rows: { count: { op: ">=", value: 2 } },
  where: {
    all: [
      { column: "open", op: "between", value: [1, "12345678901234567890"] },
      { not: { column: "team", op: "in", value: ["core", "infra"] } },
      {
        any: [
          { column: "opened_at", op: "older_than", value: "P2W" },
          { column: "team", op: "ends_with", value: "ops" },
        ],
      },
    ],
  },
} as const;

const KINDS = new Map<string, ColumnKind>([
  ["open", "number"],
  ["team", "text"],
  ["opened_at", "date"],
]);

const FILLED = {
  ...blankForm("ops"),
  name: "  Too many open PRs  ",
  metric: "prs-open",
  condition: { rows: "one", where: { column: "open", op: ">", value: 10 } },
};

const ALERT: Alert = {
  id: "a1",
  name: "Too many open PRs",
  metric: "prs-open",
  condition: NESTED as unknown as Alert["condition"],
  range: "P7D",
  interval_secs: 3_600,
  destination: "ops",
  enabled: false,
  revision: 3,
  state: {},
  created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z",
};

describe("checkForm", () => {
  it("sends the rule the form says, trimmed, with no window as null", () => {
    expect(checkForm(FILLED, KINDS)).toEqual({
      ok: true,
      draft: {
        name: "Too many open PRs",
        metric: "prs-open",
        condition: FILLED.condition,
        range: null,
        interval_secs: 300,
        destination: "ops",
        enabled: true,
      },
    });
  });

  it("sends an alert's nested condition back as it was read", () => {
    const checked = checkForm(formOf(ALERT), KINDS);

    expect(checked.ok && checked.draft).toEqual({
      name: ALERT.name,
      metric: ALERT.metric,
      condition: NESTED,
      range: "P7D",
      interval_secs: 3_600,
      destination: "ops",
      enabled: false,
    });
  });

  it.each([
    ["name", { name: "   " }],
    ["name", { name: "x".repeat(NAME_MAX + 1) }],
    ["name", { name: 7 }],
    ["metric", { metric: "" }],
    ["destination", { destination: "" }],
    ["range", { range: 30 }],
    ["interval_secs", { interval_secs: 1.5 }],
    ["interval_secs", { interval_secs: "300" }],
    ["enabled", { enabled: "yes" }],
    [
      "condition.where.column",
      { condition: { rows: "one", where: { column: "", op: ">", value: 1 } } },
    ],
    [
      "condition.where.value",
      {
        condition: {
          rows: "one",
          where: { column: "open", op: ">", value: "ten" },
        },
      },
    ],
    [
      "condition.where.all[0].value[1]",
      {
        condition: {
          rows: "any",
          where: { all: [{ column: "open", op: "between", value: [1, ""] }] },
        },
      },
    ],
    ...(
      [
        ["opened_at", "before", "2026-10-01T09:00"],
        ["opened_at", "before", "2026-10-01"],
        ["opened_at", "between", ["2026-10-01T09:00:00Z", ""]],
        ["opened_at", "older_than", "PD"],
        ["opened_at", "older_than", "P0D"],
        ["opened_at", "older_than", "PT1D"],
        ["team", "in", []],
      ] as const
    ).map(([column, op, value]) => [
      Array.isArray(value) && value.length === 2
        ? "condition.where.value[1]"
        : "condition.where.value",
      { condition: { rows: "one", where: { column, op, value } } },
    ]),
    ...[-1, 1.5, "3"].map((value) => [
      "condition.rows.count.value",
      {
        condition: {
          rows: { count: { op: ">", value } },
          where: { column: "open", op: ">", value: 1 },
        },
      },
    ]),
  ] as [string, Record<string, unknown>][])(
    "refuses a form with a bad %s",
    (field, change) => {
      const checked = checkForm({ ...FILLED, ...change }, KINDS);

      expect(checked.ok, `should reject: ${JSON.stringify(change)}`).toBe(
        false
      );
      expect(
        !checked.ok && Object.keys(checked.errors),
        `should name ${field}: ${JSON.stringify(change)}`
      ).toEqual([field]);
    }
  );

  it.each([
    ["opened_at", "after", "2026-10-01T09:00:00+02:00"],
    [
      "opened_at",
      "between",
      ["2026-09-01 00:00:00Z", "2026-10-01T00:00:00.500Z"],
    ],
    ["opened_at", "newer_than", "PT30M"],
    ["opened_at", "older_than", "P2W"],
    ["team", "not_in", ["core"]],
    ["team", "==", ""],
  ])("accepts a whole %s %s value", (column, op, value) => {
    const form = {
      ...FILLED,
      condition: { rows: "one", where: { column, op, value } },
    };

    expect(checkForm(form, KINDS).ok, JSON.stringify(value)).toBe(true);
  });

  it("leaves a value to the service while the column's kind is not known", () => {
    const form = {
      ...FILLED,
      condition: {
        rows: "one",
        where: { column: "open", op: ">", value: "ten" },
      },
    };

    expect(checkForm(form).ok).toBe(true);
  });

  it("says how long a name may be", () => {
    const checked = checkForm({ ...FILLED, name: "x".repeat(NAME_MAX + 1) });

    expect(!checked.ok && checked.errors.name).toBe(
      "Keep the name to 200 characters."
    );
  });

  it("counts a name's characters, not its UTF-16 units", () => {
    expect(checkForm({ ...FILLED, name: "🔔".repeat(NAME_MAX) }).ok).toBe(true);
  });
});

describe("a number typed", () => {
  it.each([
    ["10", 10],
    [" -2.5 ", -2.5],
    ["0", 0],
    ["+3", 3],
    ["1e3", 1000],
    ["-1e-3", -0.001],
    [".5", 0.5],
    ["9007199254740991", 9_007_199_254_740_991],
    ["9007199254740993", "9007199254740993"],
    ["-123456789012345678901", "-123456789012345678901"],
    ["", undefined],
    ["abc", undefined],
    ["1,5", undefined],
    ["−3", undefined],
    ["Infinity", undefined],
    ["1e400", undefined],
    ["1e21", undefined],
    ["+9007199254740993", undefined],
    ["0x10", undefined],
    ["1.2.3", undefined],
    ["1e", undefined],
    ["--1", undefined],
  ])("reads '%s' as %j", (typed, value) => {
    expect(numberOf(typed), `typed: ${JSON.stringify(typed)}`).toBe(value);
  });
});

describe("an operator on a column of another kind", () => {
  it.each([
    ["open", ">", 10, true],
    ["open", "contains", "1", false],
    ["open", "before", "2026-10-01T00:00:00Z", false],
    ["team", "==", "core", true],
    ["team", ">", 10, false],
    ["team", "older_than", "P7D", false],
    ["opened_at", "after", "2026-10-01T00:00:00Z", true],
    ["opened_at", ">", 10, false],
    ["opened_at", "in", ["x"], false],
  ])("checks %s %s against the column's kind", (column, op, value, fitting) => {
    const form = {
      ...FILLED,
      condition: { rows: "one", where: { column, op, value } },
    };
    const checked = checkForm(form, KINDS);
    const expected = fitting
      ? { ok: true }
      : { ok: false, errors: { "condition.where.op": "Not for this column." } };

    expect(checked, `${column} ${op}`).toMatchObject(expected);
    if (!checked.ok) {
      expect(
        Object.keys(checked.errors),
        `${column} ${op}: only the operator`
      ).toEqual(["condition.where.op"]);
    }
  });
});

describe("the draft as JSON text", () => {
  it.each([
    ['{"value": 12345678901234567890}', { value: "12345678901234567890" }],
    ['{"value": -9007199254740993}', { value: "-9007199254740993" }],
    ['{"value": 9007199254740991}', { value: 9_007_199_254_740_991 }],
    ['{"value": 1e21}', { value: 1e21 }],
    ['{"value": 2.5}', { value: 2.5 }],
  ])("reads %s without rounding an integer", (text, value) => {
    expect(parseDraftText(text), text).toEqual({ ok: true, value });
  });

  it("refuses an integer it cannot read exactly where the parser hides its digits", () => {
    const blind = (
      text: string,
      reviver: (key: string, value: unknown) => unknown
    ) =>
      JSON.parse(text, (key, value: unknown) => reviver(key, value)) as unknown;

    expect(parseDraftText('{"value": 12345678901234567890}', blind)).toEqual({
      ok: false,
      problem: "Put integers this large in quotes.",
    });
    expect(parseDraftText('{"value": 10}', blind)).toEqual({
      ok: true,
      value: { value: 10 },
    });
  });

  it.each([
    ['{ "name": ', "Unexpected"],
    ["[1, 2]", "An alert is an object."],
  ])("keeps the draft as it stood when %s cannot be read", (text, problem) => {
    const held = hold(FILLED);
    const retyped = retypeDraft(held, text);

    expect(retyped.document, text).toBe(held.document);
    expect(retyped.text).toBe(text);
    expect(retyped.unparsed).toContain(problem);
  });
});
