import { describe, expect, it } from "vitest";

import { conditionText, numberText, reasonText, statusText } from "./describe";

describe("describe", () => {
  it.each([
    [">", 10, "above 10"],
    [">=", 0.5, "at or above 0.5"],
    ["<", 1_000, "below 1,000"],
    ["<=", "123456789012345678901", "at or below 123,456,789,012,345,678,901"],
  ] as const)("says %s %s as '%s'", (operator, threshold, text) => {
    expect(conditionText(operator, threshold)).toBe(text);
  });

  it.each([
    [119, "119"],
    [1234567.25, "1,234,567.25"],
    ["98765432109876543210", "98,765,432,109,876,543,210"],
    ["not digits", "not digits"],
  ])("never rounds %s", (value, text) => {
    expect(numberText(value), `value: ${value}`).toBe(text);
  });

  it("says why for every reason code the service sends", () => {
    for (const code of [
      "no_rows",
      "many_rows",
      "column_missing",
      "null",
      "non_numeric",
      "incomparable",
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
    ["cancelled", "Withdrawn"],
    ["sent", "Sent"],
    ["failed", "Failed"],
  ])("names status %s as %s", (status, text) => {
    expect(statusText(status)).toBe(text);
  });
});
