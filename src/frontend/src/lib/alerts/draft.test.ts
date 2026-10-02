import { describe, expect, it } from "vitest";

import type { Alert } from "@/api/alerts-types";

import {
  NAME_MAX,
  blankForm,
  checkForm,
  formOf,
  thresholdOf,
  type AlertForm,
} from "./draft";

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
    ["interval_secs", { interval: { amount: 0, unit: "minutes" as const } }],
    ["interval_secs", { interval: { amount: 1.5, unit: "hours" as const } }],
    ["destination", { destination: "" }],
  ])("refuses a form with a bad %s", (field, change) => {
    const checked = checkForm({ ...FILLED, ...change });

    expect(checked.ok, `should reject: ${JSON.stringify(change)}`).toBe(false);
    expect(!checked.ok && Object.keys(checked.errors)).toEqual([field]);
  });
});

describe("what the form sends back", () => {
  it.each([
    ["an interval no unit holds exactly, untouched", {}, 90],
    [
      "the same interval, changed",
      { interval: { amount: 3, unit: "minutes" as const } },
      180,
    ],
  ])("sends %s as it should", (_case, change, secs) => {
    const checked = checkForm({
      ...formOf({ ...ALERT, interval_secs: 90 }),
      ...change,
    });

    expect(checked.ok && checked.draft.interval_secs).toBe(secs);
  });

  it("counts a name's characters, not its UTF-16 units", () => {
    const name = "🔔".repeat(NAME_MAX);

    expect(checkForm({ ...FILLED, name }).ok).toBe(true);
  });

  it("says why a threshold past 2^53 cannot be entered", () => {
    const checked = checkForm({
      ...FILLED,
      threshold: "123456789012345678901",
    });

    expect(!checked.ok && checked.errors.threshold).toBe(
      "This number is too large to enter here exactly."
    );
  });
});

describe("thresholdOf", () => {
  it.each([
    ["10", 10],
    [" -2.5 ", -2.5],
    ["0", 0],
    ["", undefined],
    ["abc", undefined],
    ["Infinity", undefined],
    ["123456789012345678901", undefined],
  ])("reads '%s' as %s", (typed, value) => {
    expect(thresholdOf(typed), `typed: ${typed}`).toBe(value);
  });
});
