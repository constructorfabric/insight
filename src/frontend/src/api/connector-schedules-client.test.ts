import { beforeEach, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";
import { ConnectorsApiError } from "./connectors-client";
import {
  deleteConnectorSchedule,
  getConnectorSchedule,
  putConnectorSchedule,
  type ConnectorSchedule,
} from "./connector-health-client";

const fetchMock = vi.mocked(fetchWithAuth);
const instance = {
  connector: "example/tracker",
  source_id: "main?",
  tenant_id: "example-tenant",
};
const body: ConnectorSchedule = {
  connector: instance.connector,
  source_id: instance.source_id,
  revision: 3,
  status: "pending",
};

beforeEach(() => fetchMock.mockReset());

it.each(["GET", "PUT", "DELETE"])(
  "%s encodes the instance, retains the ETag and uses If-Match for writes",
  async (method) => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify(body), { headers: { ETag: '"0003"' } })
    );
    const result =
      method === "GET"
        ? await getConnectorSchedule(instance)
        : method === "PUT"
          ? await putConnectorSchedule(instance, '"2"', ["0 3 * * *"])
          : await deleteConnectorSchedule(instance, '"2"');
    expect(result).toEqual({ schedule: body, etag: '"0003"' });
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/connectors/v1/connectors/example%2Ftracker/main%3F/schedule",
      expect.objectContaining({ method })
    );
    const init = fetchMock.mock.calls[0][1];
    expect(init?.headers).toEqual(
      method === "GET"
        ? {}
        : method === "PUT"
          ? { "If-Match": '"2"', "Content-Type": "application/json" }
          : { "If-Match": '"2"' }
    );
    expect(init?.body).toBe(
      method === "PUT" ? JSON.stringify({ crons: ["0 3 * * *"] }) : undefined
    );
  }
);

it.each([400, 412, 500])(
  "retains the %s problem and status for the editor",
  async (status) => {
    const problem = {
      type: "urn:example:invalid-schedule",
      title: "Schedule refused",
      detail: "Synthetic schedule refusal.",
      status,
      context: {
        field_violations: [{ description: "Synthetic invalid field." }],
      },
    };
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify(problem), { status })
    );
    await expect(
      putConnectorSchedule(instance, '"2"', ["invalid"])
    ).rejects.toEqual(new ConnectorsApiError(status, problem));
  }
);

it("refuses an unreadable revision instead of inventing an If-Match value", async () => {
  fetchMock.mockResolvedValue(new Response(JSON.stringify(body)));
  await expect(getConnectorSchedule(instance)).rejects.toThrow("no revision");
});

it("uses the configured connectors base for health, syncs, intensity and schedules", async () => {
  vi.stubEnv("VITE_CONNECTORS_BASE", "/custom/connectors/v1");
  vi.resetModules();
  for (const response of [
    { as_of: "2026-01-15T12:00:00Z", history_available: true, connectors: [] },
    { connector: instance.connector, syncs: [], window: 50 },
    {
      grain: "15m",
      series: "connector",
      from: "2026-01-14T12:00:00Z",
      to: "2026-01-15T12:00:00Z",
      truncated: false,
      points: [],
    },
    body,
  ]) {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify(response), { headers: { ETag: '"3"' } })
    );
  }
  try {
    const client = await import("./connector-health-client");
    const ingestion = await import("./ingestion-client");
    await client.getConnectorHealth();
    await client.getConnectorSyncs(instance);
    await ingestion.getIngestionIntensity({ grain: "15m" });
    await client.getConnectorSchedule(instance);
    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      "/custom/connectors/v1/connector-health",
      "/custom/connectors/v1/connector-health/example%2Ftracker/syncs?tenant_id=example-tenant&source_id=main%3F",
      "/custom/connectors/v1/ingestion/intensity?grain=15m",
      "/custom/connectors/v1/connectors/example%2Ftracker/main%3F/schedule",
    ]);
    expect(fetchMock).toHaveBeenCalledWith(
      "/custom/connectors/v1/connectors/example%2Ftracker/main%3F/schedule",
      expect.objectContaining({ method: "GET" })
    );
  } finally {
    vi.unstubAllEnvs();
    vi.resetModules();
  }
});

it.each(["omitted", "null"])(
  "retains %s optional schedule fields for every schedule operation",
  async (shape) => {
    const schedule: ConnectorSchedule =
      shape === "omitted"
        ? body
        : {
            ...body,
            requested: null,
            live: null,
            inherited: null,
            pending_reason: null,
          };
    fetchMock.mockImplementation(
      async () =>
        new Response(JSON.stringify(schedule), { headers: { ETag: '"3"' } })
    );
    const results = [
      await getConnectorSchedule(instance),
      await putConnectorSchedule(instance, '"2"', ["0 3 * * *"]),
      await deleteConnectorSchedule(instance, '"2"'),
    ];
    for (const result of results) {
      expect(result).toEqual({ schedule, etag: '"3"' });
    }
  }
);
