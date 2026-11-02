import { describe, expect, it } from "vitest";

import type { AppliedSchedule } from "@/api/connector-health-client";
import { describeNextRun } from "@/lib/portal/connector-health";

const NOW = new Date("2026-01-15T12:00:00Z");

function schedule(over: Partial<AppliedSchedule> = {}): AppliedSchedule {
  return {
    crons: ["0 22 * * *"],
    timezone: "Europe/Sofia",
    suspended: false,
    ...over,
  };
}

describe("the next scheduled start", () => {
  it.each<[string[], string, string]>([
    [
      ["0 22 * * *"],
      "Europe/Sofia",
      "2026-01-15 22:00 Europe/Sofia (in 8h 0m)",
    ],
    [
      ["0 10 * * *", "0 22 * * *"],
      "Europe/Sofia",
      "2026-01-15 22:00 Europe/Sofia (in 8h 0m)",
    ],
    [
      ["0 10 * * *"],
      "Pacific/Honolulu",
      "2026-01-15 10:00 Pacific/Honolulu (in 8h 0m)",
    ],
    [["@daily"], "UTC", "2026-01-16 00:00 UTC (in 12h 0m)"],
    [["@hourly"], "UTC", "2026-01-15 13:00 UTC (in 1h 0m)"],
    [["@weekly"], "UTC", "2026-01-18 00:00 UTC (in 60h 0m)"],
    [["@monthly"], "UTC", "2026-02-01 00:00 UTC (in 396h 0m)"],
    [["@yearly"], "UTC", "2027-01-01 00:00 UTC (in 8412h 0m)"],
    [["0 9 1 * 1"], "UTC", "2026-01-19 09:00 UTC (in 93h 0m)"],
    [["5/10 * * * *"], "UTC", "2026-01-15 12:05 UTC (in 5m 0s)"],
  ])("forecasts %j in %s", (crons, timezone, expected) => {
    expect(describeNextRun(schedule({ crons, timezone }), NOW)).toBe(
      `Next run ${expected}`
    );
  });

  it("uses the timezone's changed offset across daylight saving", () => {
    expect(
      describeNextRun(
        schedule({ crons: ["0 5 * * *"] }),
        new Date("2026-03-28T22:00:00Z")
      )
    ).toBe("Next run 2026-03-29 05:00 Europe/Sofia (in 4h 0m)");
  });

  it("suspension takes precedence over unreadable expressions and missing timezone", () => {
    expect(
      describeNextRun(
        schedule({ suspended: true, timezone: null, crons: ["@every 5m"] }),
        NOW
      )
    ).toBe("Suspended: no run is scheduled.");
  });

  it("a missing timezone takes precedence over unreadable expressions", () => {
    expect(
      describeNextRun(schedule({ timezone: null, crons: ["unreadable"] }), NOW)
    ).toBe(
      "Next run unknown: this schedule runs in the Argo controller's timezone, which is not recorded."
    );
  });

  it.each([
    ["unreadable"],
    ["0 22 * * *", "@every 5m"],
    ["0 0 22 * * *"],
    ["0 0 22 * * * 2027"],
    ["2027-01-01T00:00:00Z"],
    ["0 0 31 2 *"],
    [],
  ])("refuses unreadable expressions %j without throwing", (...crons) => {
    expect(describeNextRun(schedule({ crons }), NOW)).toBe(
      "Next run unknown: a recorded expression cannot be read."
    );
  });

  it("does not throw on an invalid timezone or clock", () => {
    for (const [recorded, now] of [
      [schedule({ timezone: "Example/Invalid" }), NOW],
      [schedule(), new Date(NaN)],
    ] as const) {
      expect(describeNextRun(recorded, now)).toBe(
        "Next run unknown: a recorded expression cannot be read."
      );
    }
  });
});
