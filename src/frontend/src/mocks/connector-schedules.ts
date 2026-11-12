import { http, HttpResponse } from "msw";

import type {
  AppliedSchedule,
  ConnectorHealth,
  ConnectorSchedule,
  ScheduleStatus,
} from "@/api/connector-health-client";
import { CONNECTORS_BASE } from "@/api/connectors-client";
import { nextScheduledRun } from "@/lib/portal/connector-schedule";
import { PEOPLE } from "./registry";

const CALLER = PEOPLE[0].person_id;
const PATH = `${CONNECTORS_BASE}/connectors/:connector/:sourceId/schedule`;

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
  const failures = new Map<string, 400 | 412 | 500 | 503>();
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
        live: {
          crons: ["0 3 * * *"],
          timezone: "UTC",
          origin: "default",
          suspended: false,
          observed_at: new Date().toISOString(),
        },
        inherited: { crons: ["0 3 * * *"], origin: "default" },
        status: "inherited",
        read_error: null,
        pending_reason: null,
        next_reconcile_at: null,
        reconcile_interval_secs: null,
      };
      states.set(id, state);
    }
    return state;
  }

  function reply(state: ConnectorSchedule) {
    if (state.live?.suspended && !state.read_error) state.status = "paused";
    const reconcile = state.status === "pending";
    state.next_reconcile_at = reconcile
      ? new Date(Math.ceil((Date.now() + 1) / 900_000) * 900_000).toISOString()
      : null;
    state.reconcile_interval_secs = reconcile ? 900 : null;
    return HttpResponse.json(state, {
      headers: { ETag: `"${state.revision}"` },
    });
  }

  function problem(
    status: number,
    descriptions: string[] = [],
    revision?: number
  ) {
    return HttpResponse.json(
      {
        type: "urn:example:schedule-error",
        title: "Schedule request refused",
        detail: "Synthetic schedule refusal.",
        status,
        context: {
          ...(revision === undefined ? {} : { revision }),
          field_violations: descriptions.map((description) => ({
            field: "crons",
            description,
          })),
        },
      },
      {
        status,
        headers: {
          "Content-Type": "application/problem+json",
          ...(revision === undefined ? {} : { ETag: `"${revision}"` }),
        },
      }
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
      state.pending_reason = "different_schedule";
    }
    if (!request.headers.has("If-Match")) return problem(428);
    if (request.headers.get("If-Match") !== `"${state.revision}"`)
      return problem(412, [], state.revision);
    if (failure === 503)
      return HttpResponse.json(
        {
          status: 503,
          detail: "The schedule was not changed: synthetic cluster refusal",
        },
        { status: 503 }
      );
    if (request.method === "DELETE" && state.live && !state.inherited)
      return HttpResponse.json(
        {
          status: 409,
          detail:
            "Reset requires the deployment schedule annotation; run reconcile before resetting",
        },
        { status: 409 }
      );
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
      state.pending_reason = "different_schedule";
    }
    return reply(state);
  }

  return {
    read,
    failNext(
      connector: string,
      sourceId: string,
      status: 400 | 412 | 500 | 503
    ) {
      failures.set(key(connector, sourceId), status);
    },
    setStatus(connector: string, sourceId: string, status: ScheduleStatus) {
      const state = read(connector, sourceId);
      state.status = status;
      if (state.live) state.live.suspended = status === "paused";
      state.pending_reason = status === "pending" ? "different_schedule" : null;
      if (status === "applied" && state.requested) {
        state.live = {
          crons: state.requested.crons,
          timezone: "UTC",
          origin: "api",
          suspended: false,
          observed_at: new Date().toISOString(),
        };
      }
      if (status === "inherited") {
        state.live = {
          crons: ["0 3 * * *"],
          timezone: "UTC",
          origin: "default",
          suspended: false,
          observed_at: new Date().toISOString(),
        };
      }
    },
    summary(
      connector: string,
      sourceId: string,
      live: AppliedSchedule | null
    ): Pick<ConnectorHealth, "schedule" | "schedule_request"> {
      const state = read(connector, sourceId);
      if (state.revision === 0)
        state.live = live
          ? { ...live, observed_at: new Date().toISOString() }
          : null;
      return {
        schedule: live,
        schedule_request:
          state.requested || live?.suspended
            ? {
                crons: state.requested?.crons ?? [],
                revision: state.revision,
                read_error: state.read_error ?? null,
                last_observation: state.status
                  ? {
                      status: state.live?.suspended ? "paused" : state.status,
                      observed_at: new Date().toISOString(),
                    }
                  : null,
              }
            : null,
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
