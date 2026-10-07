import { describe, expect, it } from "vitest";

import type { Alert } from "@/api/alerts-types";

import { blankForm, checkForm, formOf, thresholdOf, type AlertForm } from "./draft";

/** The longest name the service accepts, as the form's message states it. */
const NAME_MAX = 200;

const FILLED: AlertForm = {
  ...blankForm("ops"),
  name: "  Too many open PRs  ",
  metric: "prs-open",
  column: "total",
  threshold: "10",
};

const ALERT: Alert = {
  id: "a1",
  name: "Too many open PRs",
  metric: "prs-open",
  column: "total",
  operator: ">=",
  threshold: 10,
  range: "P7D",
  interval_secs: 3_600,
  destination: "ops",
  enabled: false,
  revision: 3,
  state: {},
  created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z",
};

/** The form an alert opens in, with its threshold field reading `typed`. */
function retyped(threshold: Alert["threshold"], typed: string): AlertForm {
  return { ...formOf({ ...ALERT, threshold }), threshold: typed };
}

describe("checkForm", () => {
  it("sends the rule the form says, trimmed, with no window as null", () => {
    expect(checkForm(FILLED)).toEqual({
      ok: true,
      draft: {
        name: "Too many open PRs",
        metric: "prs-open",
        column: "total",
        operator: ">",
        threshold: 10,
        range: null,
        interval_secs: 300,
        destination: "ops",
        enabled: true,
      },
    });
  });

  it("reads an alert back into the form it would be edited in", () => {
    const checked = checkForm(formOf(ALERT));

    expect(checked.ok && checked.draft).toEqual({
      name: ALERT.name,
      metric: ALERT.metric,
      column: ALERT.column,
      operator: ALERT.operator,
      threshold: 10,
      range: "P7D",
      interval_secs: 3_600,
      destination: "ops",
      enabled: false,
    });
  });

  it.each([
    ["name", { name: "   " }],
    ["name", { name: "x".repeat(NAME_MAX + 1) }],
    ["metric", { metric: "" }],
    ["column", { column: "" }],
    ["threshold", { threshold: "" }],
    ["threshold", { threshold: "ten" }],
    ["destination", { destination: "" }],
  ])("refuses a form with a bad %s", (field, change) => {
    const checked = checkForm({ ...FILLED, ...change });

    expect(checked.ok, `should reject: ${JSON.stringify(change)}`).toBe(false);
    expect(!checked.ok && Object.keys(checked.errors)).toEqual([field]);
  });

  it("says how long a name may be", () => {
    const checked = checkForm({ ...FILLED, name: "x".repeat(NAME_MAX + 1) });

    expect(!checked.ok && checked.errors.name).toBe(
      "Keep the name to 200 characters."
    );
  });
});

describe("what the form sends back", () => {
  it("sends a stored interval back exactly, even one that is no preset", () => {
    const checked = checkForm(formOf({ ...ALERT, interval_secs: 90 }));

    expect(checked.ok && checked.draft.interval_secs).toBe(90);
  });

  it("counts a name's characters, not its UTF-16 units", () => {
    const name = "🔔".repeat(NAME_MAX);

    expect(checkForm({ ...FILLED, name }).ok).toBe(true);
  });
});

describe("a threshold the service sent", () => {
  it.each([
    ["a digit string wider than a JavaScript number", "12345678901234567890"],
    ["an integer past 2^53", 9_007_199_254_740_992],
    ["a float in exponent form", 1e21],
    ["a plain integer", 10],
    ["a negative fraction", -2.5],
  ])("goes back as it came while the field is not retyped: %s", (_case, stored) => {
    const checked = checkForm(formOf({ ...ALERT, threshold: stored }));

    expect(checked.ok && checked.draft.threshold, `stored: ${String(stored)}`).toBe(
      stored
    );
  });

  it("goes back as it came with only spaces typed around it", () => {
    const checked = checkForm(
      retyped("12345678901234567890", " 12345678901234567890 ")
    );

    expect(checked.ok && checked.draft.threshold).toBe("12345678901234567890");
  });

  it("gives way to what is retyped", () => {
    const checked = checkForm(retyped("12345678901234567890", "11"));

    expect(checked.ok && checked.draft.threshold).toBe(11);
  });

  it("does not make a retyped integer past 2^53 acceptable", () => {
    const checked = checkForm(
      retyped("12345678901234567890", "12345678901234567891")
    );

    expect(checked).toEqual({
      ok: false,
      errors: { threshold: "Number is too large." },
    });
  });
});

describe("a threshold typed", () => {
  it.each([
    ["10", 10],
    [" -2.5 ", -2.5],
    ["0", 0],
    ["+3", 3],
    [" 7 ", 7],
    ["1e3", 1000],
    ["-1e-3", -0.001],
    [".5", 0.5],
    ["9007199254740991", 9_007_199_254_740_991],
    ["-9007199254740991", -9_007_199_254_740_991],
    ["", undefined],
    ["abc", undefined],
    ["1,5", undefined],
    ["−3", undefined],
    ["Infinity", undefined],
    ["1e400", undefined],
    ["1e21", undefined],
    ["9007199254740993", undefined],
    ["-9007199254740993", undefined],
    ["123456789012345678901", undefined],
    ["0x10", undefined],
    ["0b101", undefined],
    ["1.2.3", undefined],
    ["1e", undefined],
    ["--1", undefined],
  ])("reads '%s' as %s", (typed, value) => {
    expect(thresholdOf(typed), `typed: ${JSON.stringify(typed)}`).toBe(value);
  });

  it.each([
    ["9007199254740993", "Number is too large."],
    ["123456789012345678901", "Number is too large."],
    ["1e21", "Number is too large."],
    ["1e400", "Enter a number."],
    ["1,5", "Enter a number."],
    ["−3", "Enter a number."],
    ["", "Enter a number."],
  ])("explains why '%s' is refused", (typed, message) => {
    const checked = checkForm({ ...FILLED, threshold: typed });

    expect(!checked.ok && checked.errors.threshold, `typed: ${typed}`).toBe(
      message
    );
  });
});
