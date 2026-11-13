import { expect, it } from "vitest";

import type {
  ConnectorSchedule,
  ScheduleRequestState,
} from "@/api/connector-health-client";
import { scheduleTooltipLines } from "./connector-schedule-tooltip";
import { scheduleTime } from "./connector-schedule-time";

const schedule: ConnectorSchedule = {
  connector: "example-tracker",
  source_id: "main",
  revision: 3,
  status: "pending",
  requested: {
    crons: ["0 6 * * *"],
    updated_by: "00000000-0000-4000-8000-000000000001",
    updated_at: "2026-01-15T13:00:00+01:00",
  },
  live: {
    crons: ["0 3 * * *", "0 15 * * *"],
    observed_at: "2026-01-15T11:00:00Z",
    timezone: "UTC",
    origin: "api",
    suspended: false,
  },
  inherited: { crons: ["0 2 * * *", "0 14 * * *"], origin: "default" },
};

it.each([
  ["annotation", "the Secret annotation insight.cyberfabric.com/schedule"],
  ["descriptor", "the connector descriptor"],
  ["default", "the default"],
  ["api", "api"],
] as const)(
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
] as const)(
  "names the applied request and overridden deployment origin %s",
  (origin, source) => {
    expect(
      scheduleTooltipLines({
        ...schedule,
        status: "applied",
        live: schedule.live,
        inherited: { ...schedule.inherited!, origin },
      }),
      origin
    ).toEqual([
      `Request revision 3 is running, checked at ${scheduleTime("2026-01-15T11:00:00Z")}.`,
      "0 3 * * * UTC",
      "0 15 * * * UTC",
      `Overrides the deployment schedule from ${source}.`,
      "0 2 * * * UTC",
      "0 14 * * * UTC",
    ]);
  }
);

it("dates the pending request in UTC and names the live crons", () => {
  expect(scheduleTooltipLines({ ...schedule })).toEqual([
    `Request revision 3, saved by another admin at ${scheduleTime("2026-01-15T12:00:00Z")}.`,
    `Runs now, checked at ${scheduleTime("2026-01-15T11:00:00Z")}.`,
    "0 3 * * * UTC",
    "0 15 * * * UTC",
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
          live: missing,
        }),
        status
      ).toEqual([
        ...(status === "applied"
          ? ["The live schedule has not been recorded yet."]
          : []),
        "The deployment schedule has not been recorded yet.",
      ]);
    }
    expect(
      scheduleTooltipLines({
        ...schedule,
        status: "pending",
        live: missing,
      })
    ).toEqual([
      `Request revision 3, saved by another admin at ${scheduleTime("2026-01-15T12:00:00Z")}.`,
    ]);
    expect(
      scheduleTooltipLines({
        ...schedule,
        requested: missing,
        live: missing,
      })
    ).toEqual(["Request revision 3."]);
  }
);

it.each([
  ["no_cronworkflow", "This instance has no sync schedule yet."],
  ["awaiting_reconcile", "The sync job is not yet prepared."],
  ["different_schedule", "The sync job runs a different schedule."],
] as const)(
  "explains why a saved request is pending: %s",
  (pending_reason, wording) => {
    expect(scheduleTooltipLines({ ...schedule, pending_reason })).toContain(
      wording
    );
  }
);

it.each([
  ["pending", "Waiting for reconcile"],
  ["applied", "Applied"],
  ["inherited", "Default"],
  ["paused", "Paused"],
] as const)(
  "names the summary's %s status while schedule details load",
  (status, label) => {
    const request: ScheduleRequestState = {
      crons: ["0 6 * * *"],
      revision: 3,
      last_observation: { status, observed_at: "2026-01-15T12:00:00Z" },
    };
    expect(scheduleTooltipLines(request)).toEqual([
      `${label}. Schedule details are loading.`,
    ]);
  }
);

it("shows loading while a summary has no schedule observation", () => {
  expect(
    scheduleTooltipLines({ crons: [], revision: 3, last_observation: null })
  ).toEqual(["The schedule has not been loaded yet."]);
});

it.each(["you", "Example Admin", "another admin"])(
  "uses the supplied author wording %s without exposing the person id",
  (person) => {
    const lines = scheduleTooltipLines(schedule, person).join(" ");
    expect(lines).toContain(`saved by ${person} at`);
    expect(lines).not.toContain(schedule.requested!.updated_by);
  }
);
