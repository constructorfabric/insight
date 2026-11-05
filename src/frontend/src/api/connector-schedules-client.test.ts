import { beforeEach, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";
import { AnalyticsApiError } from "./analytics-client";
import {
  deleteConnectorSchedule,
  getConnectorSchedule,
  putConnectorSchedule,
} from "./connector-health-client";

const fetchMock = vi.mocked(fetchWithAuth);
const instance = {
  connector: "example/tracker",
  source_id: "main?",
  tenant_id: "example-tenant",
};
const body = { revision: 3, status: "pending" };

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
      "/api/analytics/v1/connectors/example%2Ftracker/main%3F/schedule",
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
    ).rejects.toEqual(new AnalyticsApiError(status, problem));
  }
);

it("refuses an unreadable revision instead of inventing an If-Match value", async () => {
  fetchMock.mockResolvedValue(new Response(JSON.stringify(body)));
  await expect(getConnectorSchedule(instance)).rejects.toThrow("no revision");
});
