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
  it("skips a start in the missing spring hour", () => {
    expect(
      describeNextRun(
        schedule({ crons: ["30 2 * * *"], timezone: "Europe/Berlin" }),
        new Date("2026-03-28T23:00:00Z")
      )
    ).toBe("Next run 2026-03-30 02:30 Europe/Berlin (in 25h 30m)");
  });

  it.each([
    ["2026-10-25T00:30:00Z", "1h 0m"],
    ["2026-10-25T00:40:00Z", "50m 0s"],
    ["2026-10-25T01:10:00Z", "20m 0s"],
  ])(
    "includes the second start in the repeated autumn hour from %s",
    (now, relative) => {
      expect(
        describeNextRun(
          schedule({ crons: ["30 2 * * *"], timezone: "Europe/Berlin" }),
          new Date(now)
        )
      ).toBe(`Next run 2026-10-25 02:30 Europe/Berlin (in ${relative})`);
    }
  );

  it("does not invent a start one hour after an ordinary match", () => {
    expect(
      describeNextRun(
        schedule({ crons: ["30 2 * * *"], timezone: "Europe/Berlin" }),
        new Date("2026-10-24T00:40:00Z")
      )
    ).toBe("Next run 2026-10-25 02:30 Europe/Berlin (in 23h 50m)");
  });

  it.each([
    [
      "0,30 2 * * *",
      "Europe/Berlin",
      "2026-10-25T00:40:00Z",
      "2026-10-25 02:00 Europe/Berlin (in 20m 0s)",
    ],
    [
      "45 1 * * *",
      "Australia/Lord_Howe",
      "2026-04-04T14:50:00Z",
      "2026-04-05 01:45 Australia/Lord_Howe (in 25m 0s)",
    ],
  ])(
    "includes every repeated start for %s in %s",
    (expression, timezone, now, expected) => {
      expect(
        describeNextRun(
          schedule({ crons: [expression], timezone }),
          new Date(now)
        )
      ).toBe(`Next run ${expected}`);
    }
  );

  it.each([
    ["0 14 * * *", "0 13 * * *"],
    ["0 13 * * *", "0 14 * * *"],
  ])("takes the earliest future start in order %j", (...crons) => {
    expect(describeNextRun(schedule({ crons, timezone: "UTC" }), NOW)).toBe(
      "Next run 2026-01-15 13:00 UTC (in 1h 0m)"
    );
  });

  it("a wildcard day of month leaves the weekday restricted to Mondays", () => {
    expect(
      describeNextRun(schedule({ crons: ["0 0 * * 1"], timezone: "UTC" }), NOW)
    ).toBe("Next run 2026-01-19 00:00 UTC (in 84h 0m)");
  });

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
    [["@annually"], "UTC", "2027-01-01 00:00 UTC (in 8412h 0m)"],
    [["@midnight"], "UTC", "2026-01-16 00:00 UTC (in 12h 0m)"],
    [["0 9 ? * WED"], "UTC", "2026-01-21 09:00 UTC (in 141h 0m)"],
    [["0 9 * JUL WED"], "UTC", "2026-07-01 09:00 UTC (in 4005h 0m)"],
    [["0 9-11/2 * * MON,WED"], "UTC", "2026-01-19 09:00 UTC (in 93h 0m)"],
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
    ["0 0 L * *"],
    ["0 0 1W * *"],
    ["0 0 * * MON#2"],
    ["0 0 * * 5L"],
    ["0 0 LW * *"],
    ["0 22 * * *", "0 0 L * *"],
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
