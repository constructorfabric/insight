import { describe, expect, it } from "vitest";

import type { AppliedSchedule } from "@/api/connector-health-client";
import { describeNextRun } from "@/lib/portal/connector-health";
import { nextScheduledRun } from "@/lib/portal/connector-schedule";

const NOW = new Date("2026-01-15T12:00:00Z");

function schedule(over: Partial<AppliedSchedule> = {}): AppliedSchedule {
  return {
    crons: ["0 22 * * *"],
    timezone: "Europe/Sofia",
    suspended: false,
    ...over,
  };
}

function expectStart(
  recorded: AppliedSchedule,
  now: Date,
  expected: string,
  text: string
): void {
  expect(describeNextRun(recorded, now)).toEqual({ text, detail: null });

  const starts = recorded.crons.map((cron) =>
    nextScheduledRun(cron, recorded.timezone!, now)?.toISOString()
  );
  expect(starts).not.toContain(undefined);
  expect(starts.sort()[0]).toBe(expected);
}

describe("the next scheduled start", () => {
  it("skips a start in the missing spring hour", () => {
    expectStart(
      schedule({ crons: ["30 2 * * *"], timezone: "Europe/Berlin" }),
      new Date("2026-03-28T23:00:00Z"),
      "2026-03-30T00:30:00.000Z",
      "in ~1 d"
    );
  });

  it.each([
    ["2026-10-25T00:30:00Z", "in ~1 h"],
    ["2026-10-25T00:40:00Z", "in ~50 min"],
    ["2026-10-25T01:10:00Z", "in ~20 min"],
  ])(
    "includes the second start in the repeated autumn hour from %s",
    (now, relative) => {
      expectStart(
        schedule({ crons: ["30 2 * * *"], timezone: "Europe/Berlin" }),
        new Date(now),
        "2026-10-25T01:30:00.000Z",
        relative
      );
    }
  );

  it("does not invent a start one hour after an ordinary match", () => {
    expectStart(
      schedule({ crons: ["30 2 * * *"], timezone: "Europe/Berlin" }),
      new Date("2026-10-24T00:40:00Z"),
      "2026-10-25T00:30:00.000Z",
      "in ~1 d"
    );
  });

  it.each([
    [
      "0,30 2 * * *",
      "Europe/Berlin",
      "2026-10-25T00:40:00Z",
      "2026-10-25T01:00:00.000Z",
      "in ~20 min",
    ],
    [
      "45 1 * * *",
      "Australia/Lord_Howe",
      "2026-04-04T14:50:00Z",
      "2026-04-04T15:15:00.000Z",
      "in ~25 min",
    ],
  ])(
    "includes every repeated start for %s in %s",
    (expression, timezone, now, expected, text) => {
      expectStart(
        schedule({ crons: [expression], timezone }),
        new Date(now),
        expected,
        text
      );
    }
  );

  it.each([
    ["0 14 * * *", "0 13 * * *"],
    ["0 13 * * *", "0 14 * * *"],
  ])("takes the earliest future start in order %j", (...crons) => {
    expectStart(
      schedule({ crons, timezone: "UTC" }),
      NOW,
      "2026-01-15T13:00:00.000Z",
      "in ~1 h"
    );
  });

  it("a wildcard day of month leaves the weekday restricted to Mondays", () => {
    expectStart(
      schedule({ crons: ["0 0 * * 1"], timezone: "UTC" }),
      NOW,
      "2026-01-19T00:00:00.000Z",
      "in ~4 d"
    );
  });

  it.each<[string[], string, string, string]>([
    [["0 22 * * *"], "Europe/Sofia", "2026-01-15T20:00:00.000Z", "in ~8 h"],
    [
      ["0 10 * * *", "0 22 * * *"],
      "Europe/Sofia",
      "2026-01-15T20:00:00.000Z",
      "in ~8 h",
    ],
    [["0 10 * * *"], "Pacific/Honolulu", "2026-01-15T20:00:00.000Z", "in ~8 h"],
    [["@daily"], "UTC", "2026-01-16T00:00:00.000Z", "in ~12 h"],
    [["@hourly"], "UTC", "2026-01-15T13:00:00.000Z", "in ~1 h"],
    [["@weekly"], "UTC", "2026-01-18T00:00:00.000Z", "in ~3 d"],
    [["@monthly"], "UTC", "2026-02-01T00:00:00.000Z", "in ~17 d"],
    [["@yearly"], "UTC", "2027-01-01T00:00:00.000Z", "in ~351 d"],
    [["@annually"], "UTC", "2027-01-01T00:00:00.000Z", "in ~351 d"],
    [["@midnight"], "UTC", "2026-01-16T00:00:00.000Z", "in ~12 h"],
    [["0 9 ? * WED"], "UTC", "2026-01-21T09:00:00.000Z", "in ~6 d"],
    [["0 9 * JUL WED"], "UTC", "2026-07-01T09:00:00.000Z", "in ~167 d"],
    [["0 9-11/2 * * MON,WED"], "UTC", "2026-01-19T09:00:00.000Z", "in ~4 d"],
    [["0 9 1 * 1"], "UTC", "2026-01-19T09:00:00.000Z", "in ~4 d"],
    [["5/10 * * * *"], "UTC", "2026-01-15T12:05:00.000Z", "in ~5 min"],
  ])("forecasts %j in %s", (crons, timezone, expected, text) => {
    expectStart(schedule({ crons, timezone }), NOW, expected, text);
  });

  it("uses the timezone's changed offset across daylight saving", () => {
    expectStart(
      schedule({ crons: ["0 5 * * *"] }),
      new Date("2026-03-28T22:00:00Z"),
      "2026-03-29T02:00:00.000Z",
      "in ~4 h"
    );
  });

  it("suspension takes precedence over unreadable expressions and missing timezone", () => {
    expect(
      describeNextRun(
        schedule({ suspended: true, timezone: null, crons: ["@every 5m"] }),
        NOW
      )
    ).toEqual({ text: "Suspended", detail: "Suspended: no run is scheduled." });
  });

  it("a missing timezone takes precedence over unreadable expressions", () => {
    expect(
      describeNextRun(schedule({ timezone: null, crons: ["unreadable"] }), NOW)
    ).toEqual({
      text: "Unknown",
      detail:
        "Next run unknown: this schedule runs in the Argo controller's timezone, which is not recorded.",
    });
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
    expect(describeNextRun(schedule({ crons }), NOW)).toEqual({
      text: "Unknown",
      detail: "Next run unknown: a recorded expression cannot be read.",
    });
  });

  it("does not throw on an invalid timezone or clock", () => {
    for (const [recorded, now] of [
      [schedule({ timezone: "Example/Invalid" }), NOW],
      [schedule(), new Date(NaN)],
    ] as const) {
      expect(describeNextRun(recorded, now)).toEqual({
        text: "Unknown",
        detail: "Next run unknown: a recorded expression cannot be read.",
      });
    }
  });
});

describe("the next start uses a coarse interval", () => {
  it.each([
    [1, "in < 1 min"],
    [59_999, "in < 1 min"],
    [60_000, "in ~1 min"],
    [89_999, "in ~1 min"],
    [90_000, "in ~2 min"],
    [3_569_999, "in ~59 min"],
    [3_570_000, "in ~1 h"],
    [3_585_000, "in ~1 h"],
    [3_599_999, "in ~1 h"],
    [3_600_000, "in ~1 h"],
    [5_399_999, "in ~1 h"],
    [5_400_000, "in ~2 h"],
    [84_599_999, "in ~23 h"],
    [84_600_000, "in ~1 d"],
    [85_800_000, "in ~1 d"],
    [86_399_999, "in ~1 d"],
    [86_400_000, "in ~1 d"],
    [129_599_999, "in ~1 d"],
    [129_600_000, "in ~2 d"],
  ])("rounds a delay of %i ms to %s", (delay, text) => {
    const start = new Date("2027-01-01T00:00:00.000Z");
    const now = new Date(start.getTime() - delay);
    expectStart(
      schedule({ crons: ["@yearly"], timezone: "UTC" }),
      now,
      start.toISOString(),
      text
    );
  });
});
