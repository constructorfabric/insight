import { expect, it } from "vitest";

import type {
  ConnectorSchedule,
  ScheduleRequestState,
} from "@/api/connector-health-client";
import { scheduleTooltipLines } from "./connector-schedule-tooltip";

const schedule: ConnectorSchedule = {
  connector: "example-tracker",
  source_id: "main",
  revision: 3,
  status: "pending",
  requested: {
    crons: ["0 6 * * *"],
    updated_by: "user@example.com",
    updated_at: "2026-01-15T13:00:00+01:00",
  },
  applied: {
    crons: ["0 3 * * *", "0 15 * * *"],
    revision: 2,
    observed_at: "2026-01-15T11:00:00Z",
    timezone: "UTC",
    origin: "api",
    suspended: false,
  },
  inherited: { crons: ["0 2 * * *", "0 14 * * *"], origin: "default" },
  error: {
    attempted_revision: 3,
    attempted_at: "2026-01-15T11:45:00Z",
    message: "Synthetic workflow update failed.",
  },
};

it.each([
  ["annotation", "the Secret annotation insight.cyberfabric.com/schedule"],
  ["descriptor", "the connector descriptor"],
  ["default", "the default"],
  ["example-origin", "example-origin"],
])(
  "names the inherited deployment origin %s and every cron",
  (origin, source) => {
    expect(
      scheduleTooltipLines({
        ...schedule,
        status: "inherited",
        requested: null,
        inherited: { ...schedule.inherited!, origin },
      }),
      origin
    ).toEqual([
      `No schedule request. Runs the deployment schedule from ${source}.`,
      "0 2 * * * UTC",
      "0 14 * * * UTC",
    ]);
  }
);

it.each([
  ["annotation", "the Secret annotation insight.cyberfabric.com/schedule"],
  ["descriptor", "the connector descriptor"],
  ["default", "the default"],
])(
  "names the applied request and overridden deployment origin %s",
  (origin, source) => {
    expect(
      scheduleTooltipLines({
        ...schedule,
        status: "applied",
        applied: { ...schedule.applied!, revision: 3 },
        inherited: { ...schedule.inherited!, origin },
      }),
      origin
    ).toEqual([
      "Request revision 3 is running, checked at 2026-01-15 11:00:00Z.",
      "0 3 * * * UTC",
      "0 15 * * * UTC",
      `Overrides the deployment schedule from ${source}.`,
      "0 2 * * * UTC",
      "0 14 * * * UTC",
    ]);
  }
);

it("dates the pending request in UTC and names the older running crons", () => {
  expect(scheduleTooltipLines(schedule)).toEqual([
    "Request revision 3, saved by user@example.com at 2026-01-15 12:00:00Z.",
    "Revision 2 is running now.",
    "0 3 * * * UTC",
    "0 15 * * * UTC",
  ]);
});

it("dates the actual failed attempt independently of the latest request revision", () => {
  expect(
    scheduleTooltipLines({ ...schedule, revision: 4, status: "error" })
  ).toEqual([
    "Applying request revision 3 failed at 2026-01-15 11:45:00Z: Synthetic workflow update failed.",
    "Revision 2 is running.",
  ]);
});

it.each([null, undefined])(
  "handles missing records as %s without inventing facts",
  (missing) => {
    expect(scheduleTooltipLines(missing ?? undefined)).toEqual([
      "The schedule has not been loaded yet.",
    ]);
    for (const status of ["inherited", "applied"] as const) {
      expect(
        scheduleTooltipLines({
          ...schedule,
          status,
          inherited: missing,
          applied: missing,
        }),
        status
      ).toEqual([
        ...(status === "applied"
          ? ["The applied schedule has not been recorded yet."]
          : []),
        "The deployment schedule has not been recorded yet.",
      ]);
    }
    expect(
      scheduleTooltipLines({
        ...schedule,
        status: "error",
        applied: missing,
        error: missing,
      })
    ).toEqual(["The failed schedule attempt has not been recorded yet."]);
    expect(
      scheduleTooltipLines({
        ...schedule,
        requested: missing,
        applied: missing,
      })
    ).toEqual(["Request revision 3."]);
  }
);

it.each([2, 3, 4])(
  "only describes an older applied revision as running while pending or errored: %s",
  (revision) => {
    for (const status of ["pending", "error"] as const) {
      const lines = scheduleTooltipLines({
        ...schedule,
        status,
        applied: { ...schedule.applied!, revision },
      });
      expect(
        lines.some((line) => line.startsWith("Revision ")),
        status
      ).toBe(revision < schedule.revision);
      expect(lines[0], status).toContain(
        status === "pending"
          ? "Request revision 3"
          : "Applying request revision 3"
      );
    }
  }
);

it("uses summary facts", () => {
  const request: ScheduleRequestState = {
    crons: ["0 6 * * *"],
    status: "pending",
    revision: 3,
    updated_by: null,
    updated_at: null,
    applied: null,
  };
  expect(scheduleTooltipLines(request)).toEqual([
    "Request revision 3, saved by — at —.",
  ]);
});

it("omits saved-by attribution from a clear summary", () => {
  const request: ScheduleRequestState = {
    crons: [],
    status: "pending",
    revision: 3,
    updated_by: "user@example.com",
    updated_at: "2026-01-15T12:00:00Z",
  };
  expect(scheduleTooltipLines(request)).toEqual(["Request revision 3."]);
});
