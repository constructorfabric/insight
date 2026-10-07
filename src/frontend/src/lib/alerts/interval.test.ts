import { describe, expect, it } from "vitest";

import { intervalLabel } from "./interval";

describe("interval", () => {
  it.each([
    [60, "1 minute"],
    [300, "5 minutes"],
    [3_600, "1 hour"],
    [5_400, "90 minutes"],
    [86_400, "1 day"],
    [604_800, "7 days"],
    [90, "90 seconds"],
  ])("says %i seconds exactly in the largest unit that holds it", (secs, label) => {
    expect(intervalLabel(secs), `label: ${secs}`).toBe(label);
  });
});
