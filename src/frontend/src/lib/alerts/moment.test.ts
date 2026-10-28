import { afterEach, describe, expect, it } from "vitest";

import { momentParts, momentText, pickedDay, pickerDate } from "./moment";

const ZONE = process.env.TZ;

afterEach(() => {
  process.env.TZ = ZONE;
});

describe("momentParts", () => {
  it.each([
    ["2026-10-01T09:30:00Z", { day: "2026-10-01", time: "09:30" }],
    ["2026-10-01T22:30:15.500Z", { day: "2026-10-01", time: "22:30" }],
    ["2026-10-01T23:30:00-05:00", { day: "2026-10-02", time: "04:30" }],
    ["2026-10-01T01:00:00+14:00", { day: "2026-09-30", time: "11:00" }],
    ["2026-10-01", undefined],
    ["soon", undefined],
    [42, undefined],
  ])("reads %j as its UTC day and minute", (value, parts) => {
    expect(momentParts(value), JSON.stringify(value)).toEqual(parts);
  });
});

describe("the picked day", () => {
  it.each([
    ["UTC", 0],
    ["Pacific/Kiritimati", -840],
    ["Pacific/Pago_Pago", 660],
    ["America/Los_Angeles", 480],
    ["Asia/Kolkata", -330],
  ])("never shifts in %s", (zone, offset) => {
    process.env.TZ = zone;
    expect(new Date(2026, 0, 1).getTimezoneOffset(), `${zone} in force`).toBe(
      offset
    );

    for (const day of [
      "2026-03-08",
      "2026-10-01",
      "2026-12-31",
      "2026-01-01",
    ]) {
      const picked = pickerDate(day);
      expect(picked?.getDate(), `${zone}: ${day}`).toBe(Number(day.slice(8)));
      expect(pickedDay(picked as Date), `${zone}: ${day}`).toBe(day);
    }
  });

  it("writes a day and time back as an RFC 3339 UTC timestamp", () => {
    expect(momentText("2026-10-15", "07:05")).toBe("2026-10-15T07:05:00Z");
    expect(momentParts(momentText("2026-10-15", "07:05"))).toEqual({
      day: "2026-10-15",
      time: "07:05",
    });
  });
});
