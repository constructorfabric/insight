import { http, HttpResponse } from "msw";

import type {
  AppliedSchedule,
  ConnectorHealth,
  ConnectorSchedule,
  ScheduleStatus,
} from "@/api/connector-health-client";
import { nextScheduledRun } from "@/lib/portal/connector-schedule";
import { PEOPLE } from "./registry";

const CALLER = PEOPLE[0].person_id;
const PATH = "/api/analytics/v1/connectors/:connector/:sourceId/schedule";

function hasScheduledRun(crons: string[], now: Date): boolean {
  try {
    const starts = crons.map((cron) => nextScheduledRun(cron, "UTC", now));
    return starts.some((start) => start !== null);
  } catch {
    return false;
  }
}

export function createConnectorScheduleMocks() {
  const states = new Map<string, ConnectorSchedule>();
  const failures = new Map<string, 400 | 412 | 500>();
  const key = (connector: string, sourceId: string) =>
    `${connector}/${sourceId}`;

  function read(connector: string, sourceId: string): ConnectorSchedule {
    const id = key(connector, sourceId);
    let state = states.get(id);
    if (!state) {
      state = {
        connector,
        source_id: sourceId,
        revision: 0,
        requested: null,
        applied: {
          crons: ["0 3 * * *"],
          timezone: "UTC",
          origin: "default",
          suspended: false,
          revision: 0,
          observed_at: new Date().toISOString(),
        },
        inherited: { crons: ["0 3 * * *"], origin: "default" },
        error: null,
        status: "inherited",
      };
      states.set(id, state);
    }
    return state;
  }

  function reply(state: ConnectorSchedule) {
    return HttpResponse.json(state, {
      headers: { ETag: `"${state.revision}"` },
    });
  }

  function problem(status: number, descriptions: string[] = []) {
    return HttpResponse.json(
      {
        status,
        context: {
          field_violations: descriptions.map((description) => ({
            field: "crons",
            description,
          })),
        },
      },
      { status }
    );
  }

  async function write(request: Request, connector: string, sourceId: string) {
    const state = read(connector, sourceId);
    const failure = failures.get(key(connector, sourceId));
    failures.delete(key(connector, sourceId));
    if (failure === 412) {
      state.revision++;
      state.requested = {
        crons: ["0 9 * * *"],
        updated_by: "00000000-0000-4000-8000-000000000002",
        updated_at: new Date().toISOString(),
      };
      state.status = "pending";
    }
    if (request.headers.get("If-Match") !== `"${state.revision}"`)
      return problem(412);
    if (failure)
      return problem(failure, [
        "Cron minute field must be a number, range, list, step or *. Replay with a valid UTC expression.",
      ]);
    let crons: string[] = [];
    if (request.method === "PUT") {
      const body = (await request.json()) as { crons?: unknown };
      if (
        !Array.isArray(body.crons) ||
        body.crons.length === 0 ||
        body.crons.length > 10 ||
        body.crons.some((cron) => typeof cron !== "string")
      )
        return problem(400, ["Supply one to ten UTC cron expressions."]);
      crons = body.crons as string[];
      if (!hasScheduledRun(crons, new Date()))
        return problem(400, [
          "A cron expression cannot be read or never runs.",
        ]);
    }
    if (
      JSON.stringify(state.requested?.crons ?? []) !== JSON.stringify(crons)
    ) {
      state.revision++;
      state.requested = {
        crons,
        updated_by: CALLER,
        updated_at: new Date().toISOString(),
      };
      state.status = "pending";
    }
    return reply(state);
  }

  return {
    read,
    failNext(connector: string, sourceId: string, status: 400 | 412 | 500) {
      failures.set(key(connector, sourceId), status);
    },
    setStatus(connector: string, sourceId: string, status: ScheduleStatus) {
      const state = read(connector, sourceId);
      state.status = status;
      state.error =
        status === "error"
          ? {
              attempted_revision: state.revision,
              attempted_at: new Date().toISOString(),
              message: "Synthetic schedule apply failure.",
            }
          : null;
      if (status === "applied" && state.requested) {
        state.applied = {
          crons: state.requested.crons,
          timezone: "UTC",
          origin: "api",
          suspended: false,
          revision: state.revision,
          observed_at: new Date().toISOString(),
        };
      }
      if (status === "inherited") {
        state.applied = {
          crons: ["0 3 * * *"],
          timezone: "UTC",
          origin: "default",
          suspended: false,
          revision: state.revision,
          observed_at: new Date().toISOString(),
        };
      }
    },
    summary(
      connector: string,
      sourceId: string,
      applied: AppliedSchedule | null
    ): Pick<ConnectorHealth, "schedule" | "schedule_request"> {
      const state = read(connector, sourceId);
      if (state.revision === 0)
        state.applied = applied
          ? {
              ...applied,
              origin: applied.origin ?? "",
              revision: state.revision,
              observed_at: new Date().toISOString(),
            }
          : null;
      return {
        schedule: applied,
        schedule_request: {
          crons: state.requested?.crons ?? [],
          revision: state.revision,
          status: state.status,
          updated_at: state.requested?.updated_at ?? null,
          updated_by: state.requested?.updated_by ?? null,
          applied: state.applied,
          inherited: state.inherited,
          error: state.error,
        },
      };
    },
    handlers: [
      http.get(PATH, ({ params }) =>
        reply(read(String(params.connector), String(params.sourceId)))
      ),
      http.put(PATH, ({ request, params }) =>
        write(request, String(params.connector), String(params.sourceId))
      ),
      http.delete(PATH, ({ request, params }) =>
        write(request, String(params.connector), String(params.sourceId))
      ),
    ],
  };
}

export const connectorScheduleMocks = createConnectorScheduleMocks();
