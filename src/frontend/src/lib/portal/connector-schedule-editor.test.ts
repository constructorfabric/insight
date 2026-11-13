import { expect, it } from "vitest";

import {
  HOUR_CHOICES,
  choiceCrons,
  choiceSummary,
  cronTranslation,
  detectChoice,
  scheduleWords,
  type ScheduleChoice,
} from "./connector-schedule-editor";

it.each(HOUR_CHOICES)("detects the compiled %s-hour UTC preset", (hours) => {
  const choice = { kind: "hours", hours, time: "00:17" } as const;
  expect(detectChoice(choiceCrons(choice))).toEqual(choice);
});

it.each([
  ["01:00", "0 1 * * *", "0 1 * * 1-5"],
  ["23:30", "30 23 * * *", "30 23 * * 1-5"],
  ["08:45", "45 8 * * *", "45 8 * * 1-5"],
] as const)(
  "compiles daily and weekday UTC presets at %s",
  (time, daily, weekdays) => {
    for (const kind of ["daily", "weekdays"] as const) {
      const choice: ScheduleChoice = { kind, time };
      const expected = [kind === "daily" ? daily : weekdays];
      expect(choiceCrons(choice), kind).toEqual(expected);
      expect(detectChoice(expected), kind).toEqual(choice);
      expect(scheduleWords(expected), kind).toContain(time);
    }
  }
);

it.each([
  [1, "15 * * * *"],
  [2, "15 1,3,5,7,9,11,13,15,17,19,21,23 * * *"],
  [3, "15 0,3,6,9,12,15,18,21 * * *"],
  [4, "15 1,5,9,13,17,21 * * *"],
  [6, "15 3,9,15,21 * * *"],
  [8, "15 1,9,17 * * *"],
  [12, "15 9,21 * * *"],
] as const)(
  "compiles every %s hours with an explicit UTC phase",
  (hours, expected) => {
    expect(choiceCrons({ kind: "hours", hours, time: "09:15" })).toEqual([
      expected,
    ]);
  }
);

it("keeps unmatched ordered rows custom without interpreting or splitting their text", () => {
  for (const crons of [
    ["0 3 * * *", "0 6 * * *"],
    ["0 6 * * *", "0 3 * * *"],
    ["0 */6 * * *"],
    ["00 03 * * *"],
    ["0 24 * * *"],
    ["15 * * * *", "15 * * * *"],
  ]) {
    expect(detectChoice(crons), crons.join("; ")).toEqual({
      kind: "custom",
      crons,
    });
  }
  expect(
    choiceCrons({
      kind: "custom",
      crons: [" 0 3 * * * ", "", "0 6 * * *;0 7 * * *"],
    })
  ).toEqual([" 0 3 * * * ", "", "0 6 * * *;0 7 * * *"]);
});

it.each([
  ["0 7 * * *", "At 07:00 UTC"],
  ["0 7 * * 1-5", "At 07:00, Monday through Friday UTC"],
  ["unreadable", null],
] as const)("translates %s without validating it", (cron, expected) => {
  expect(cronTranslation(cron)).toBe(expected);
});

it.each([
  [{ kind: "hours", hours: 1, time: "00:15" }, "Syncs every hour at :15 UTC."],
  [
    { kind: "hours", hours: 6, time: "05:00" },
    "Syncs every 6 hours: 05:00, 11:00, 17:00, 23:00 UTC.",
  ],
  [{ kind: "daily", time: "09:00" }, "Syncs every day at 09:00 UTC."],
  [{ kind: "weekdays", time: "09:00" }, "Syncs Monday to Friday at 09:00 UTC."],
  [{ kind: "custom", crons: ["0 7 * * *"] }, "At 07:00 UTC"],
  [
    { kind: "custom", crons: ["0 7 * * *", "0 8 * * *"] },
    "Syncs on 2 expressions (all UTC).",
  ],
] satisfies [ScheduleChoice, string][])("summarizes %j", (choice, expected) => {
  expect(choiceSummary(choice)).toBe(expected);
});
