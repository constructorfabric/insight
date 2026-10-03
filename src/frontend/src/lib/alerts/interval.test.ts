import { describe, expect, it } from "vitest";

import { intervalLabel, intervalText } from "./interval";

describe("interval", () => {
  it.each([
    [60, "1 minute", "Every minute"],
    [300, "5 minutes", "Every 5 minutes"],
    [3_600, "1 hour", "Every hour"],
    [5_400, "90 minutes", "Every 90 minutes"],
    [86_400, "1 day", "Every day"],
    [604_800, "7 days", "Every 7 days"],
    [90, "90 seconds", "Every 90 seconds"],
  ])("says %i seconds exactly", (secs, label, text) => {
    expect(intervalLabel(secs), `label: ${secs}`).toBe(label);
    expect(intervalText(secs), `text: ${secs}`).toBe(text);
  });
});
