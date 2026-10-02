import { describe, expect, it } from "vitest";

import { fromSeconds, intervalText, toSeconds } from "./interval";

describe("interval", () => {
  it.each([
    [60, { amount: 1, unit: "minutes" }],
    [300, { amount: 5, unit: "minutes" }],
    [3_600, { amount: 1, unit: "hours" }],
    [5_400, { amount: 90, unit: "minutes" }],
    [86_400, { amount: 1, unit: "days" }],
    [604_800, { amount: 7, unit: "days" }],
    [90, { amount: 2, unit: "minutes" }],
  ] as const)(
    "reads %i seconds in the largest unit that holds them",
    (secs, input) => {
      expect(fromSeconds(secs), `seconds: ${secs}`).toEqual(input);
    }
  );

  it.each([60, 300, 3_600, 5_400, 86_400, 604_800])(
    "round-trips %i seconds through the form",
    (secs) => {
      expect(toSeconds(fromSeconds(secs)), `seconds: ${secs}`).toBe(secs);
    }
  );

  it.each([
    [60, "Every minute"],
    [300, "Every 5 minutes"],
    [3_600, "Every hour"],
    [7_200, "Every 2 hours"],
    [86_400, "Every day"],
    [604_800, "Every 7 days"],
    [30, "Every 30 seconds"],
  ])("says %i seconds as %s", (secs, text) => {
    expect(intervalText(secs)).toBe(text);
  });
});
